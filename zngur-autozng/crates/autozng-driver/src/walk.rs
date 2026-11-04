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

/// The crate that defines a type the walk may descend into.
///
/// Every non-local crate costs one external hop, except the standard library: the first
/// `std`/`core`/`alloc` type on a branch is free. That keeps a re-exported crate's API together
/// with the standard-library types its signatures name (`Option`, `Result`, `Vec`, ...) even at
/// `max_external_depth = 1`, without also paying for the generic wrapper chains those types
/// expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkTarget {
    /// Not an ADT (`str`, a slice, a tuple, a `dyn Trait`, a function pointer, ...): never
    /// charges a hop.
    NonAdt,
    /// Defined by the analyzed crate.
    Local,
    /// Defined by `std`, `core`, or `alloc`.
    Std,
    /// Defined by another crate.
    External,
}

impl WalkTarget {
    /// Classify `ty` by the crate that defines its ADT.
    pub fn of(ty: Ty) -> Self {
        let Some(adt) = ty.adt_def() else {
            return WalkTarget::NonAdt;
        };
        let krate = adt.krate();
        if krate.is_local {
            WalkTarget::Local
        } else if matches!(krate.name.as_str(), "std" | "core" | "alloc") {
            WalkTarget::Std
        } else {
            WalkTarget::External
        }
    }
}

/// The depth budget copied into a walk. Never read from `Config` during the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBudget {
    pub remaining_depth: usize,
    pub remaining_external: usize,
    /// The cap for a child whose unwrapped type is already in the walk's ancestor chain.
    pub self_ref_limit: usize,
    /// Whether the one free `std`/`core`/`alloc` hop on this branch has been spent.
    pub std_hop_used: bool,
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
            std_hop_used: false,
        }
    }

    /// The budget for a child of the type this budget describes.
    pub fn child(&self, target: WalkTarget, is_cyclic: bool) -> Self {
        // The first entry into the standard library on this branch is free; every later hop,
        // including `std` -> `std`, is charged like any other external crate.
        let (charged, std_hop_used) = match target {
            WalkTarget::Std if !self.std_hop_used => (0, true),
            WalkTarget::Std | WalkTarget::External => (1, self.std_hop_used),
            WalkTarget::Local | WalkTarget::NonAdt => (0, self.std_hop_used),
        };
        Self {
            remaining_depth: if is_cyclic {
                self.remaining_depth
                    .saturating_sub(1)
                    .min(self.self_ref_limit)
            } else {
                self.remaining_depth.saturating_sub(1)
            },
            remaining_external: self.remaining_external.saturating_sub(charged),
            self_ref_limit: self.self_ref_limit,
            std_hop_used,
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

impl WalkParts {
    /// Whether every known interface part has been walked.
    ///
    /// A recorded type starts with no parts (see [`WalkState::seen`]); [`WalkParts::all`] is only
    /// reached once a full interface walk has finished. A partial record (for example an
    /// `emit_items` method selection) can therefore still be upgraded to a full walk.
    pub fn all_walked(self) -> bool {
        self.contains(WalkParts::all())
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
    /// Whether the strongest budget had already spent its free `std`/`core`/`alloc` hop.
    pub best_std_hop_used: bool,
    pub strongest_reason: VisitReason,
    pub seen_by_value: bool,
    pub filter: WalkFilter,
}

impl WalkState {
    /// A type recorded directly by the crate walk: its children are walked outside the
    /// interface queue, and [`crate::ctx::AutozngContext::mark_walked`] records the parts the walk
    /// actually visited.
    pub fn seen(ty: Ty) -> Self {
        Self {
            ty,
            intent: WalkIntent::Walk,
            walked: WalkParts::empty(),
            best_remaining_depth: 0,
            best_remaining_external: 0,
            best_std_hop_used: true,
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
            best_std_hop_used: true,
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
            best_std_hop_used: request.budget.std_hop_used,
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
    /// Whether a `walk_filter` attribute created this filter.
    ///
    /// The built-in defaults — no attribute at all, and the `All` an explicit `emit` /
    /// `emit_items` uses — leave this `false`, so the type-specific defaults for slices and
    /// `str` still apply. A filter written by the user sets it and replaces those defaults.
    explicit: bool,
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
    /// The owner type's **defining** crate (`adt.krate().name()`), not its `.zng` path.
    Crate(String),
    /// A prefix of the owner type's **public** `.zng` path, matchable segment-wise. `crate` is
    /// the analyzed crate; a re-exported external type keeps its re-export path.
    PathPrefix(Vec<String>),
    /// The owner type is defined outside the analyzed crate.
    External,
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

/// The type an interface item belongs to, for the owner-based `Crate` / `PathPrefix` /
/// `External` rules.
#[derive(Debug, Clone, Default)]
pub struct WalkOwner {
    /// The public `.zng` path segments, without a leading `::`. A re-exported external type keeps
    /// its re-export path (`crate::...`); `crate` means the analyzed crate.
    pub path: Option<Vec<String>>,
    /// The defining crate (`"std"`, `"png"`, ...), not the re-export path.
    pub krate: Option<String>,
    /// Whether the owner is defined outside the analyzed crate.
    pub is_external: bool,
}

/// The interface item being considered by a filter.
#[derive(Debug, Clone, Copy)]
pub struct WalkItem<'a> {
    pub kind: WalkItemKind,
    pub name: Option<&'a str>,
    pub receiver: Option<ReceiverKind>,
    pub visibility: Option<VisibilityKind>,
    pub trait_path: Option<&'a str>,
    /// The type the item belongs to; `None` for an item with no owner (a free function).
    pub owner: Option<&'a WalkOwner>,
}

impl Default for WalkFilter {
    fn default() -> Self {
        Self {
            roots: vec![WalkFilterRoot::All],
            explicit: false,
        }
    }
}

impl WalkFilter {
    pub fn new(roots: Vec<WalkFilterRoot>) -> Self {
        Self {
            roots,
            explicit: true,
        }
    }

    /// Whether a `walk_filter` attribute created this filter.
    pub fn is_explicit(&self) -> bool {
        self.explicit
    }

    /// The default inherent-method filter for a slice (`[T]`) or `str`.
    ///
    /// These interfaces are large and mostly iterator/closure API a `.zng` interface cannot use,
    /// so only the essentials are walked by default. An explicit `walk_filter` on the selecting
    /// item replaces this list; `emit` / `emit_items` on its own does not (it would otherwise
    /// silently bring back the whole API).
    pub fn default_for_inherent_methods(ty: Ty) -> Option<Self> {
        let names: &[&str] = match ty.base_ty().kind() {
            TyKind::RigidTy(RigidTy::Slice(_)) => &[
                "len",
                "is_empty",
                "as_ptr",
                "as_mut_ptr",
                "contains",
                "to_vec",
            ],
            TyKind::RigidTy(RigidTy::Str) => &[
                "len",
                "is_empty",
                "from_utf8",
                "as_bytes",
                "as_bytes_mut",
                "contains",
            ],
            _ => return None,
        };
        Some(Self {
            roots: vec![WalkFilterRoot::Include(
                names
                    .iter()
                    .map(|name| WalkRule::Named((*name).to_owned()))
                    .collect(),
            )],
            explicit: false,
        })
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
            WalkRule::Crate(krate) => {
                item.owner.and_then(|owner| owner.krate.as_deref()) == Some(krate.as_str())
            }
            WalkRule::PathPrefix(prefix) => item
                .owner
                .and_then(|owner| owner.path.as_deref())
                .is_some_and(|path| path.starts_with(prefix)),
            WalkRule::External => item.owner.is_some_and(|owner| owner.is_external),
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
            owner: None,
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

    #[test]
    fn crate_path_and_external_rules_match_the_owner() {
        let local = WalkOwner {
            path: Some(vec!["crate".into(), "nested".into(), "Thing".into()]),
            krate: Some("example_crate".into()),
            is_external: false,
        };
        let local_item = WalkItem {
            owner: Some(&local),
            ..item(WalkItemKind::Field, Some("value"))
        };

        let by_crate = |krate: &str| {
            WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::Crate(
                krate.into(),
            )])])
        };
        assert!(by_crate("example_crate").allows(&local_item));
        assert!(!by_crate("std").allows(&local_item));

        let by_path = |segments: &[&str]| {
            WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::PathPrefix(
                segments.iter().map(|s| (*s).to_string()).collect(),
            )])])
        };
        assert!(by_path(&["crate", "nested"]).allows(&local_item));
        assert!(!by_path(&["crate", "other"]).allows(&local_item));

        let external = WalkFilter::new(vec![WalkFilterRoot::Include(vec![WalkRule::External])]);
        assert!(!external.allows(&local_item));
        let external_owner = WalkOwner {
            is_external: true,
            ..local.clone()
        };
        assert!(external.allows(&WalkItem {
            owner: Some(&external_owner),
            ..local_item
        }));
    }
}
