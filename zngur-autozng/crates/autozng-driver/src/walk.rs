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

use rustc_public::CrateDef;
use rustc_public::ty::{GenericArgKind, RigidTy, Ty, TyKind};
use rustc_span::Span;
use smallvec::SmallVec;

use crate::rustc_ext::RustcPublicTyExt;

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
    /// The cap for a child whose unwrapped type is already in the walk's ancestor chain.
    pub self_ref_limit: usize,
}

impl WalkBudget {
    pub fn root(
        max_walk_depth: usize,
        max_external_depth: usize,
        max_self_ref_depth: usize,
    ) -> Self {
        Self {
            remaining_depth: max_walk_depth,
            remaining_external: max_external_depth,
            self_ref_limit: max_self_ref_depth,
        }
    }

    /// The budget for a child of the type this budget describes.
    pub fn child(&self, is_external: bool, is_cyclic: bool) -> Self {
        Self {
            remaining_depth: if is_cyclic {
                self.remaining_depth
                    .saturating_sub(1)
                    .min(self.self_ref_limit)
            } else {
                self.remaining_depth.saturating_sub(1)
            },
            remaining_external: if is_external {
                self.remaining_external.saturating_sub(1)
            } else {
                self.remaining_external
            },
            self_ref_limit: self.self_ref_limit,
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
    /// Unwrapped base keys from the root down to the parent, used to detect cycles.
    pub ancestors: SmallVec<[Ty; 8]>,
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
    /// How the owner itself is used; a field of a by-reference owner is by reference.
    pub requested_as: RequestedAs,
    /// Unwrapped base keys from the root down to this type.
    pub ancestors: SmallVec<[Ty; 8]>,
}

/// The innermost non-wrapper type.
///
/// One-argument wrappers are stripped to a fixpoint, so `Option<&T>`, `Vec<T>`, and `T` compare
/// equal when looking for a self-referential expansion. This is deliberately conservative: it
/// also treats a `Vec<T>` field on `T` as self-referential, which is the intended cap for
/// recursive data structures.
pub fn base_ty(ty: Ty) -> Ty {
    let mut current = ty.base_ty();
    while let Some(inner) = unwrap_one(current) {
        current = inner.base_ty();
    }
    current
}

fn unwrap_one(ty: Ty) -> Option<Ty> {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Ref(_, inner, _))
        | TyKind::RigidTy(RigidTy::RawPtr(inner, _))
        | TyKind::RigidTy(RigidTy::Slice(inner))
        | TyKind::RigidTy(RigidTy::Array(inner, _)) => Some(inner),
        TyKind::RigidTy(RigidTy::Adt(adt, args)) if args.0.len() == 1 => {
            let name = adt.name();
            let is_wrapper = matches!(
                name.as_str(),
                "std::boxed::Box"
                    | "alloc::boxed::Box"
                    | "std::rc::Rc"
                    | "alloc::rc::Rc"
                    | "std::sync::Arc"
                    | "alloc::sync::Arc"
                    | "std::vec::Vec"
                    | "alloc::vec::Vec"
                    | "std::option::Option"
                    | "core::option::Option"
                    | "std::pin::Pin"
                    | "core::pin::Pin"
                    | "std::cell::Cell"
                    | "core::cell::Cell"
                    | "std::cell::RefCell"
                    | "core::cell::RefCell"
                    | "std::mem::MaybeUninit"
                    | "core::mem::MaybeUninit"
                    | "std::num::Wrapping"
                    | "core::num::Wrapping"
            );
            if is_wrapper && let GenericArgKind::Type(inner) = &args.0[0] {
                Some(*inner)
            } else {
                None
            }
        }
        _ => None,
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkItemKind {
    Field,
    Variant,
    InherentMethod,
    TraitMethod,
}

/// The interface item being considered by a filter.
#[derive(Debug, Clone, Copy)]
pub struct WalkItem<'a> {
    pub kind: WalkItemKind,
    pub name: Option<&'a str>,
    pub receiver: Option<ReceiverKind>,
    pub visibility: Option<VisibilityKind>,
    pub trait_path: Option<&'a str>,
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

    /// Whether the walk may visit `item`.
    ///
    /// Roots are evaluated in source order. Each matching root sets the verdict (`all` /
    /// `include` allow, `none` / `exclude` deny) and the last match wins. With no match, the
    /// default is the inverse of the first root: `include` / `none` deny, `exclude` / `all`
    /// allow.
    pub fn allows(&self, item: &WalkItem<'_>) -> bool {
        let mut verdict = match self.roots.first() {
            Some(WalkFilterRoot::Include(_)) | Some(WalkFilterRoot::None) => false,
            Some(WalkFilterRoot::Exclude(_)) | Some(WalkFilterRoot::All) => true,
            None => true,
        };
        for root in &self.roots {
            match root {
                WalkFilterRoot::All => verdict = true,
                WalkFilterRoot::None => verdict = false,
                WalkFilterRoot::Include(rules) if rules.iter().any(|rule| rule.matches(item)) => {
                    verdict = true;
                }
                WalkFilterRoot::Exclude(rules) if rules.iter().any(|rule| rule.matches(item)) => {
                    verdict = false;
                }
                WalkFilterRoot::Include(_) | WalkFilterRoot::Exclude(_) => {}
            }
        }
        verdict
    }
}

impl WalkRule {
    fn matches(&self, item: &WalkItem<'_>) -> bool {
        match self {
            WalkRule::Fields => item.kind == WalkItemKind::Field,
            WalkRule::Variants => item.kind == WalkItemKind::Variant,
            WalkRule::InherentMethods => item.kind == WalkItemKind::InherentMethod,
            WalkRule::TraitMethods(path) => {
                item.kind == WalkItemKind::TraitMethod
                    && path
                        .as_deref()
                        .is_none_or(|path| item.trait_path == Some(path.trim_start_matches("::")))
            }
            WalkRule::Named(name) => item.name == Some(name.as_str()),
            WalkRule::Receiver(receiver) => item.receiver == Some(*receiver),
            WalkRule::Visibility(visibility) => item.visibility == Some(*visibility),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<'a>(kind: WalkItemKind, name: Option<&'a str>) -> WalkItem<'a> {
        WalkItem {
            kind,
            name,
            receiver: None,
            visibility: None,
            trait_path: None,
        }
    }

    #[test]
    fn a_single_include_is_an_allow_list() {
        let filter = WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::Fields])]);
        assert!(filter.allows(&item(WalkItemKind::Field, Some("a"))));
        assert!(!filter.allows(&item(WalkItemKind::InherentMethod, Some("m"))));
    }

    #[test]
    fn a_single_exclude_is_a_deny_list() {
        let filter = WalkFilter::new(vec![WalkFilterRoot::Exclude(vec![WalkRule::Named(
            "x".into(),
        )])]);
        assert!(!filter.allows(&item(WalkItemKind::Field, Some("x"))));
        assert!(filter.allows(&item(WalkItemKind::Field, Some("y"))));
    }

    #[test]
    fn the_last_matching_root_wins() {
        let filter = WalkFilter::new(vec![
            WalkFilterRoot::Include(vec![WalkRule::Fields]),
            WalkFilterRoot::Exclude(vec![WalkRule::Named("x".into())]),
        ]);
        assert!(filter.allows(&item(WalkItemKind::Field, Some("a"))));
        assert!(!filter.allows(&item(WalkItemKind::Field, Some("x"))));

        let reversed = WalkFilter::new(vec![
            WalkFilterRoot::Exclude(vec![WalkRule::Named("x".into())]),
            WalkFilterRoot::Include(vec![WalkRule::Fields]),
        ]);
        assert!(reversed.allows(&item(WalkItemKind::Field, Some("x"))));
    }

    #[test]
    fn receiver_visibility_and_trait_path_are_matched() {
        let receiver = WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::Receiver(
            ReceiverKind::Value,
        )])]);
        let by_value = WalkItem {
            receiver: Some(ReceiverKind::Value),
            ..item(WalkItemKind::InherentMethod, Some("take"))
        };
        assert!(receiver.allows(&by_value));

        let visibility =
            WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::Visibility(
                VisibilityKind::Public,
            )])]);
        let public = WalkItem {
            visibility: Some(VisibilityKind::Public),
            ..item(WalkItemKind::Field, Some("a"))
        };
        assert!(visibility.allows(&public));

        let trait_path =
            WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::TraitMethods(
                Some("::core::fmt::Debug".into()),
            )])]);
        let debug = WalkItem {
            kind: WalkItemKind::TraitMethod,
            trait_path: Some("core::fmt::Debug"),
            ..item(WalkItemKind::TraitMethod, Some("fmt"))
        };
        assert!(trait_path.allows(&debug));
    }
}
