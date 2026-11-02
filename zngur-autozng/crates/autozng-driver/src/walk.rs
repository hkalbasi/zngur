//! Walk filters: which interface items a recursive interface walk may visit.
//!
//! A filter is attached with `#[autozng::walk_filter(...)]` on the crate root or on an
//! emit-able item. It gates both the emission of an interface item and the walking of its
//! type. A nested filter replaces the inherited filter for its whole subtree (see
//! [`crate::attrs::AutozngAttrsExt`] and `Attr::for_child`).
//!
//! A filter is an ordered list of roots:
//!
//! ```text
//! walk_filter(exclude(TraitMethods), include(Fields, InherentMethods))
//! ```
//!
//! Each matching root sets the verdict (`all` / `include` allow, `none` / `exclude` deny) and
//! the **last matching root wins**. When no root matches, the default is the inverse of the
//! first root: `include` / `none` deny, `exclude` / `all` allow.

use rustc_public::ty::Ty;
use rustc_span::Span;

/// Whether a request walks a type's interface or only declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkIntent {
    /// Emit the interface item and walk its children.
    Walk,
    /// Emit an opaque block; never walk the children.
    Declare,
}

/// Why a type was requested for the interface walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitReason {
    ExplicitEmit,
    EmitItems,
    DefaultVisibility,
    Reexport,
    Field,
    MethodSignature,
    TraitMethod,
    EmitTrait,
    EmitOnImplementers,
    ConcreteMethodSignature,
    AsGenericArg,
}

impl VisitReason {
    /// Ordering used when several paths reach the same type: a stronger reason can upgrade a
    /// declaration to a walk.
    pub fn precedence(self) -> u8 {
        match self {
            VisitReason::ExplicitEmit | VisitReason::EmitItems => 4,
            VisitReason::DefaultVisibility | VisitReason::Reexport => 3,
            VisitReason::Field
            | VisitReason::MethodSignature
            | VisitReason::TraitMethod
            | VisitReason::EmitTrait
            | VisitReason::EmitOnImplementers
            | VisitReason::ConcreteMethodSignature => 2,
            VisitReason::AsGenericArg => 0,
        }
    }
}

/// Which analysis pass produced a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkPhase {
    CrateWalk,
    Reexport,
    InterfaceDrain,
    ConcreteSignature,
}

/// How the requesting position uses the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedAs {
    Value,
    Ref,
    RefMut,
    Receiver,
    GenericArg,
}

/// The depth budget copied into a walk. Never read from `Config` during the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBudget {
    pub remaining_depth: usize,
    pub remaining_external: usize,
}

impl WalkBudget {
    pub fn root(max_walk_depth: usize, max_external_depth: usize) -> Self {
        Self {
            remaining_depth: max_walk_depth,
            remaining_external: max_external_depth,
        }
    }

    /// The budget for a child of the type this budget describes.
    pub fn child(&self, is_external: bool) -> Self {
        Self {
            remaining_depth: self.remaining_depth.saturating_sub(1),
            remaining_external: if is_external {
                self.remaining_external.saturating_sub(1)
            } else {
                self.remaining_external
            },
        }
    }

    /// Whether a type at this budget may walk its fields and methods.
    pub fn exhausted(&self) -> bool {
        self.remaining_depth == 0 || self.remaining_external == 0
    }
}

/// One entry in the interface visit queue.
#[derive(Debug, Clone)]
pub struct WalkRequest {
    pub ty: Ty,
    /// The region-erased identity used for dedupe and merge.
    pub key: Ty,
    pub reason: VisitReason,
    pub phase: WalkPhase,
    pub budget: WalkBudget,
    pub requested_as: RequestedAs,
    pub span: Span,
    pub parent: Option<Ty>,
    pub filter: WalkFilter,
}

/// The walk currently in progress, used to derive a child request.
#[derive(Debug, Clone)]
pub struct WalkContext {
    /// The type whose fields/methods are being walked.
    pub owner: Ty,
    pub phase: WalkPhase,
    pub budget: WalkBudget,
    pub filter: WalkFilter,
    pub span: Span,
}

bitflags::bitflags! {
    /// Interface parts already visited for a type.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct WalkParts: u8 {
        const VARIANTS = 0b0001;
        const FIELDS = 0b0010;
        const INHERENT = 0b0100;
        const TRAIT = 0b1000;
    }
}

/// What is known about a type reached during the walk.
#[derive(Debug, Clone)]
pub struct WalkState {
    pub ty: Ty,
    pub intent: WalkIntent,
    pub walked: WalkParts,
    pub best_remaining_depth: usize,
    pub best_remaining_external: usize,
    pub strongest_reason: VisitReason,
    pub seen_by_value: bool,
    pub filter: WalkFilter,
}

impl WalkState {
    /// A type recorded directly by the crate walk: its interface is handled outside the
    /// interface queue.
    pub fn seen(ty: Ty) -> Self {
        Self {
            ty,
            intent: WalkIntent::Walk,
            walked: WalkParts::all(),
            best_remaining_depth: 0,
            best_remaining_external: 0,
            strongest_reason: VisitReason::DefaultVisibility,
            seen_by_value: true,
            filter: WalkFilter::default(),
        }
    }

    /// A declaration-only type, e.g. a lifetime-bearing signature type or a truncated block.
    pub fn declare(ty: Ty, reason: VisitReason, filter: WalkFilter, walked: WalkParts) -> Self {
        Self {
            ty,
            intent: WalkIntent::Declare,
            walked,
            best_remaining_depth: 0,
            best_remaining_external: 0,
            strongest_reason: reason,
            seen_by_value: false,
            filter,
        }
    }

    /// A type queued for a full interface walk.
    pub fn requested(ty: Ty, request: &WalkRequest) -> Self {
        Self {
            ty,
            intent: WalkIntent::Walk,
            walked: WalkParts::empty(),
            best_remaining_depth: request.budget.remaining_depth,
            best_remaining_external: request.budget.remaining_external,
            strongest_reason: request.reason,
            seen_by_value: request.requested_as == RequestedAs::Value,
            filter: request.filter.clone(),
        }
    }
}

/// An ordered list of filter roots. The default is `all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkFilter {
    roots: Vec<WalkFilterRoot>,
}

/// One `all` / `none` / `include(...)` / `exclude(...)` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkFilterRoot {
    /// Allow every item.
    All,
    /// Deny every item.
    None,
    /// Allow items matched by at least one rule.
    Include(Vec<WalkRule>),
    /// Deny items matched by at least one rule.
    Exclude(Vec<WalkRule>),
}

/// One selector inside an `include(...)` / `exclude(...)` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkRule {
    Fields,
    Variants,
    InherentMethods,
    /// Trait methods, optionally restricted to one trait path.
    TraitMethods(Option<String>),
    Named(String),
    Receiver(ReceiverKind),
    Visibility(VisibilityKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    Value,
    Ref,
    RefMut,
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityKind {
    Public,
    PublicInternal,
    Private,
}

impl Default for WalkFilter {
    fn default() -> Self {
        Self {
            roots: vec![WalkFilterRoot::All],
        }
    }
}

impl WalkFilter {
    pub fn new(roots: Vec<WalkFilterRoot>) -> Self {
        Self { roots }
    }
}
