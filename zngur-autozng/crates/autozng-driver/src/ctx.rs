//! The accumulation target for the analysis passes.
//!
//! [`AutozngContext`] collects one [`TyRecord`] per visited type (keyed by `Ty`), the set of
//! free functions selected for emission, plus a [`MetadataMap`] of per-item tool attributes
//! keyed by `MetadataKey`. Both passes in `analyze.rs` (the HIR pass and the
//! `rustc_public` pass) write into the same context.
//!
//! A type's fields reach it through [`AutozngContext::record_field`] and its methods through
//! [`AutozngContext::record_impl_method`] (both called while walking the ADT in `analyze.rs`);
//! free functions through [`AutozngContext::record_fn`].

use indexmap::{IndexMap, IndexSet};

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_public::CrateDef;
use rustc_public::CrateDefType;
use rustc_public::abi::Layout;
use rustc_public::mir::Mutability;
use rustc_public::ty::{
    AdtDef, FieldDef, FnDef, FnSig, GenericArgKind, GenericArgs, GenericParamDefKind, ImplDef,
    RigidTy, TraitDecl, TraitDef, TraitRef, TyKind,
};
use rustc_public::{DefId, ty::Ty};
use rustc_span::Span;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::config::Config;

use crate::attrs::attributes::rename::TraitRenameSpec;
use crate::driver::with_ctxt;
use crate::error::{emit_warning_for_span, span_of_def};
use crate::metadata::{
    ExportName, IntoMetadataKey, LayoutOverride, MetadataKey, MetadataMap, MetadataRecordRefMut,
};
use crate::rustc_ext::{
    ClausesOfExt, FnDefExt, GenericClauses, GetAssocInfoExt, RustcInternalEx, RustcPublicTyExt,
    StabilityExt, Visibility, VisibilityExt, fn_trait_name, stable_trait, visible_path,
};
use crate::type_map::{is_type_block_less, map_rustc_public_ty_to_zngur};
use crate::walk::{
    ReceiverKind, RequestedAs, VisibilityKind, VisitReason, WalkBudget, WalkContext, WalkFilter,
    WalkIntent, WalkItem, WalkItemKind, WalkParts, WalkPhase, WalkRequest, WalkState, WalkTarget,
    base_ty,
};

#[derive(Debug)]
pub struct FnRecord {
    pub def: FnDef,
    pub clauses: GenericClauses,
    /// The item's *own* generic arguments, when they were explicitly enumerated.
    ///
    /// A `let` binding inside an `#[autozng::emit_items]` const that names a monomorphization
    /// (`let _ = foo::<u32>;`, or `let _ = Self::bar::<u32>;` inside an impl) records the named
    /// generic args here. For a method on a generic impl the impl's `Self` parameters are
    /// unknown at that point, so the method cannot be instantiated until a fully concrete `Self`
    /// (say `Generic<REAL_TYPE>`) is being emitted, at which point `Generic<REAL_TYPE>::bar::<u32>`
    /// is what gets written. Either way the type is still validated against `clauses` on
    /// emission. `None` for every item recorded by the ordinary impl / crate walk.
    pub partial_args: Option<GenericArgs>,
    /// Span of the `#[autozng::emit]` that selected this item (see [`TyRecord::span`]).
    pub span: Span,
}

impl PartialEq for FnRecord {
    fn eq(&self, other: &Self) -> bool {
        // The span is not part of the record's identity: the same method recorded from two
        // `emit` sites must still deduplicate (see the `IndexSet`s that hold these).
        self.def == other.def
            && self.clauses == other.clauses
            && self.partial_args == other.partial_args
    }
}
impl Eq for FnRecord {}

impl Hash for FnRecord {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.def.hash(state);
    }
}

/// A method of a *trait*, attached to one concrete implementing type.
///
/// Recorded when an `emit_items` const names a trait method (`let _ = Example::to_string;`, or a
/// user trait through its impl: `<Example as SomeTrait>::method`), which the impl walk cannot see
/// because trait visiting is stubbed. Emitted as a method of the self type, with the trait as the
/// `use_path` (`fn to_string(&self) -> ::std::string::String use ::std::string::ToString;`), and
/// only if the trait's clauses hold for the concrete arguments.
#[derive(Debug)]
pub struct TraitFnRecord {
    /// The trait method itself (e.g. `ToString::to_string`).
    pub def: FnDef,
    /// the the Trait associated with this function
    pub trait_def: TraitDef,
    /// Trait will qualified generic args like the `Self` of the implementing type
    /// impls that the function is form an impl block and not just the definition from the trait decl
    #[allow(dead_code)]
    // consumed once trait rendering lands; currently only `trait_def` is read
    pub trait_ref: Option<TraitRef>,
    /// The method's generic args, `Self` first: the ones named by the `let` binding's `FnDef`
    /// type, or the method's identity args for a trait declaration.
    pub generics: GenericArgs,
    /// The trait method's own clauses.
    pub clauses: GenericClauses,
    /// Span of the item that selected this method (an `#[autozng::emit_items]` const's `let`
    /// binding, or the method declaration itself for a trait declaration; see [`TyRecord::span`]).
    pub span: Span,
}

impl PartialEq for TraitFnRecord {
    fn eq(&self, other: &Self) -> bool {
        self.def == other.def && self.generics == other.generics
    }
}
impl Eq for TraitFnRecord {}

impl Hash for TraitFnRecord {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.def.hash(state);
    }
}

/// One exported trait declaration (`#[autozng::emit]` on a `pub`/`pub(crate)` trait).
///
/// The trait analogue of [`TyRecord`]: it carries the trait's declaration, its clauses and the
/// associated functions selected for emission. A trait is only recorded when it carries an
/// explicit `emit` (its implicit action is `None`, see [`rustc_public::ty::TraitDef`]'s
/// `EmitTarget`), and every associated function of an emitted trait is recorded because a
/// trait's functions are at most as visible as the trait itself.
///
/// A `TraitRef` (i.e. `Self: Trait<...>`) is a function-level notion and lives on each
/// [`TraitFnRecord`], not here.
// `decl`/`fns` drive rendering; the recorded spans/clauses are not consumed yet.
#[allow(dead_code)]
#[derive(Debug)]
pub struct ExportedTraitRecord {
    /// The trait's declaration (the stable `TraitDecl`; see `rustc_ext::stable_trait`).
    pub decl: TraitDecl,
    /// Span of the `#[autozng::emit]` that selected the trait.
    pub span: Span,
    /// Source span of the trait declaration (the primary span for its diagnostics).
    pub source_span: Span,
    /// The trait's own clauses.
    pub clauses: GenericClauses,
    /// The trait's associated functions (methods), in declaration order.
    pub fns: IndexSet<TraitFnRecord>,
    /// The concrete `Self: Trait<Args>` reference when the trait was selected as a *concrete*
    /// instantiation by `#[autozng::emit_items(emit_traits = true)]` naming e.g.
    /// `&dyn GenericTrait<u32>`. `None` for a declaration selected by `#[autozng::emit]`;
    /// declarations cannot be generic (see [`AutozngContext::record_trait`]).
    pub concrete: Option<TraitRef>,
}

/// A trait (or a trait impl) tagged `#[autozng::emit_on_implementers]`.
///
/// The trait's functions are emitted on every type that implements the trait and satisfies
/// `clauses`, without the type having to enumerate them in an `emit_items` const. The record is
/// produced by [`AutozngContext::record_emit_for_trait`] (a trait declaration) or
/// [`AutozngContext::record_emit_for_impl`] (an impl block), and consumed while rendering a type
/// (see `render::build_emit_for_methods`).
#[derive(Debug)]
pub struct EmitForRecord {
    /// The trait whose functions should be emitted on implementers.
    pub trait_def: TraitDef,
    /// The impl block the attribute was attached to, if any (`None` for a trait declaration).
    /// Used to resolve the concrete `Self: Trait<..>` reference and the impl's clauses.
    pub impl_def: Option<ImplDef>,
    /// The clauses a candidate type must satisfy: the trait's own clauses (declaration case) or
    /// the impl block's clauses (impl case).
    pub clauses: GenericClauses,
    /// The span responsible for this record (the declaration / impl block that was tagged).
    pub span: Span,
    /// The trait's associated functions, with their identity generic arguments (`Self` first).
    pub fns: IndexSet<TraitFnRecord>,
}

/// The trait's associated functions as [`TraitFnRecord`]s with their identity generic arguments.
///
/// Shared by the `emit_on_implementers` trait-declaration and impl-block records: both record the
/// trait *declaration*'s functions (the impl supplies the clauses, not a different signature).
fn trait_fn_records(trait_def: TraitDef, span: Span) -> IndexSet<TraitFnRecord> {
    trait_def
        .associated_items()
        .into_iter()
        .filter_map(|assoc_item| {
            let (fn_def, generics) = assoc_item.opt_fn_def()?;
            Some(TraitFnRecord {
                def: fn_def,
                trait_def,
                trait_ref: None,
                generics,
                clauses: fn_def.clauses_of(),
                span,
            })
        })
        .collect()
}

#[derive(Debug)]
pub struct ImplRecord {
    pub def_id: DefId,
    pub methods: IndexSet<FnRecord>,
    pub clauses: GenericClauses,
}

impl PartialEq for ImplRecord {
    fn eq(&self, other: &Self) -> bool {
        self.def_id == other.def_id
    }
}
impl Eq for ImplRecord {}

impl Hash for ImplRecord {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.def_id.hash(state);
    }
}

/// One analyzed type: its layout/clauses plus the impls discovered for it.
#[derive(Debug)]
pub struct TyRecord {
    pub ty: Ty,
    pub adt: Option<AdtDef>,
    pub layout: Option<Layout>,
    #[allow(dead_code)] // kept for layout/clause policies that are not applied yet
    pub clauses: GenericClauses,
    pub is_concrete: bool,
    /// Span of the `#[autozng::emit]` / `emit_items` that selected this type for emission.
    ///
    /// Used by render diagnostics: it is the primary span for a type with no `DefId` of its own
    /// (e.g. an enumerated slice/tuple), and an `emitted here` note otherwise.
    pub span: Span,
    /// Fields selected for emission (via visibility + `emit`/`skip`), keyed by the field's
    /// `DefId` to deduplicate and keep declaration order.
    pub fields: IndexMap<DefId, FieldRecord>,
    /// Keyed by the impl's `DefId`; populated by [`AutozngContext::record_impl_method`].
    pub impls: IndexMap<DefId, ImplRecord>,
    /// Trait methods attached to this type (see [`TraitFnRecord`]).
    pub trait_fns: IndexSet<TraitFnRecord>,
    /// Whether an explicit `emit` / `emit_items` selected this type. Explicit selection
    /// overrides the walk filter for the type's subtree.
    pub selected_explicitly: bool,
}

/// A field selected for emission, together with the span of the `emit` that selected it.
#[derive(Debug, Clone)]
pub struct FieldRecord {
    #[allow(dead_code)] // the field is re-looked-up from the ADT at render time
    pub def: FieldDef,
    pub span: Span,
}

#[derive(Debug, Default)]
struct AutozngContextData {
    tys: RefCell<FxHashMap<Ty, TyRecord>>,
    metadata: RefCell<MetadataMap>,
    /// Free functions selected for emission, in discovery order.
    funcs: RefCell<IndexSet<FnRecord>>,
    /// Trait declarations selected for emission, in discovery order.
    traits: RefCell<Vec<ExportedTraitRecord>>,
    /// Traits / trait impls tagged `#[autozng::emit_on_implementers]`, in discovery order. Their
    /// functions are emitted on every implementing type (see [`EmitForRecord`]).
    emit_for: RefCell<Vec<EmitForRecord>>,
    /// Types exposed by an emitted interface (a field's type, or a function / method signature)
    /// that still need their own `type` block, in discovery order.
    ///
    /// Drained by the third analysis pass (see [`crate::analyze`]); visiting a type may queue
    /// further types.
    visit_queue: RefCell<VecDeque<WalkRequest>>,
    /// Every type already recorded, or already requested for a third-pass visit.
    ///
    /// Guards [`AutozngContextData::visit_queue`]: a type is queued at most once, so a cyclic
    /// interface (or a type reached through several paths) is never visited twice.
    walk_states: RefCell<FxHashMap<Ty, WalkState>>,
    /// The walk currently in progress, if any. Attribute visitors (`record_field`,
    /// `record_impl_method`, ...) read it to derive the child request's budget and provenance.
    current_walk: RefCell<Vec<WalkContext>>,
    /// Concrete instances whose instantiated method signatures still need queueing.
    pending_concrete: RefCell<VecDeque<(Ty, AdtDef)>>,
    /// Concrete instances already queued for the signature worklist.
    seen_concrete: RefCell<FxHashSet<Ty>>,
    /// Traits already rejected as not dyn compatible, so the error is emitted once per trait even
    /// when the crate walk and the re-export pass both reach it (see
    /// [`AutozngContext::ensure_trait_exportable`]).
    unexportable_traits: RefCell<Vec<TraitDef>>,
    /// `rename(trait = "...")` specs, keyed by the ADT whose trait methods they rename (see
    /// [`crate::attrs::attributes::rename::TraitRenameSpec`]).
    trait_renames: RefCell<FxHashMap<DefId, Vec<TraitRenameSpec>>>,
    /// Spans of the `trait_path` specs that matched an emitted trait method, so an unmatched
    /// selector can be warned about once.
    matched_trait_renames: RefCell<FxHashSet<Span>>,
}

impl AutozngContextData {
    fn metadata_mut_by_key(&self, key: MetadataKey) -> MetadataRecordRefMut<'_> {
        std::cell::RefMut::map(self.metadata.borrow_mut(), |metadata| {
            metadata.entry(key).or_default()
        })
    }

    fn have_ty(&self, ty: &Ty) -> bool {
        self.tys.borrow().contains_key(&canonical_ty(*ty))
    }

    /// Mark `ty` as seen, so it is never queued for a third-pass visit.
    fn mark_seen(&self, ty: Ty) {
        let ty = canonical_ty(ty);
        self.walk_states
            .borrow_mut()
            .entry(ty)
            .or_insert_with(|| WalkState::seen(ty));
    }

    fn ty_record_mut(&self, ty: Ty, span: Span) -> std::cell::RefMut<'_, TyRecord> {
        let ty = canonical_ty(ty);
        // Every type that gets a record is seen: it must not be queued for a third-pass visit.
        self.mark_seen(ty);
        let adt = ty.adt_def();
        std::cell::RefMut::map(self.tys.borrow_mut(), |tys| {
            tys.entry(ty).or_insert_with(|| TyRecord {
                ty,
                adt,
                layout: ty.layout().ok(),
                clauses: adt.map_or_default(|adt| adt.clauses_of()),
                is_concrete: ty.is_concrete(),
                span,
                fields: IndexMap::default(),
                impls: IndexMap::default(),
                trait_fns: IndexSet::default(),
                selected_explicitly: false,
            })
        })
    }
}

#[derive(Debug, Default)]
pub struct AutozngContext {
    cfg: RefCell<Config>,
    data: AutozngContextData,
}

/// Whether `ty` can be declared as its own IDL `type` block.
///
/// A numeric primitive is translated by zngur to a native C++ numeric type and the unit tuple
/// (`()`) is specialised, so neither is ever declared; a type with no IDL representation at
/// all (an array, a fn pointer, a type parameter, ...) has no block either.
fn can_have_type_block(ty: Ty) -> bool {
    match map_rustc_public_ty_to_zngur(ty) {
        Ok(rust_ty) => !is_type_block_less(&rust_ty),
        Err(_) => false,
    }
}

/// The filter's view of an item's driver visibility.
pub(crate) fn visibility_kind(visibility: Visibility) -> VisibilityKind {
    match visibility {
        Visibility::Public => VisibilityKind::Public,
        Visibility::PublicInternal => VisibilityKind::PublicInternal,
        Visibility::PrivateAPI => VisibilityKind::Private,
    }
}

/// The receiver form of a function, for `Receiver(...)` filter rules.
pub(crate) fn receiver_kind(fn_def: &FnDef) -> ReceiverKind {
    let Some(item) = fn_def.associated_item() else {
        return ReceiverKind::Static;
    };
    let rustc_public::ty::AssocKind::Fn { has_self, .. } = item.kind else {
        return ReceiverKind::Static;
    };
    if !has_self {
        return ReceiverKind::Static;
    }
    let signature = fn_def.fn_sig().skip_binder();
    match signature.inputs().first().map(|ty| ty.kind()) {
        Some(TyKind::RigidTy(RigidTy::Ref(_, _, Mutability::Not))) => ReceiverKind::Ref,
        Some(TyKind::RigidTy(RigidTy::Ref(_, _, Mutability::Mut))) => ReceiverKind::RefMut,
        _ => ReceiverKind::Value,
    }
}

/// Replace every region with `'erased` so a signature type such as `Iter<'_, T>` has one
/// canonical identity regardless of which bound lifetime it came from.
struct EraseRegions<'tcx>(rustc_middle::ty::TyCtxt<'tcx>);

impl<'tcx> rustc_middle::ty::TypeFolder<rustc_middle::ty::TyCtxt<'tcx>> for EraseRegions<'tcx> {
    fn cx(&self) -> rustc_middle::ty::TyCtxt<'tcx> {
        self.0
    }

    fn fold_region(&mut self, _: rustc_middle::ty::Region<'tcx>) -> rustc_middle::ty::Region<'tcx> {
        self.0.lifetimes.re_erased
    }
}

fn erase_regions(ty: Ty) -> Ty {
    with_ctxt(|driver| {
        use rustc_middle::ty::TypeFoldable;

        let tcx = driver.tcx;
        rustc_public::rustc_internal::stable(ty.internal(tcx).fold_with(&mut EraseRegions(tcx)))
    })
}

/// The canonical identity of `ty` for records, walk state, and queues.
///
/// Records and states are keyed by this: regions are erased (so `Info<'a>` and `Info<'_>` are
/// one type) and one-argument wrappers are unwrapped.
fn canonical_ty(ty: Ty) -> Ty {
    erase_regions(ty).base_ty()
}

/// The signature of `fn_def` with `args` applied when they cover every generic parameter of the
/// function, or its identity signature otherwise (see [`FnDefExt::instantiate_sig`]).
///
/// Used when a function / method is recorded, so the types its interface exposes can be queued
/// for a third-pass visit even for a monomorphization whose arguments are not known yet (a
/// method on a generic impl keeps its identity signature, whose type parameters are simply not
/// queueable).
fn instantiate_signature(fn_def: FnDef, args: Option<&GenericArgs>) -> FnSig {
    match args.filter(|args| args.0.len() == fn_def.generics_len()) {
        Some(args) => fn_def.instantiate_sig(args),
        None => fn_def.fn_sig().skip_binder(),
    }
}

/// Whether `fn_def` is a method with a `self` receiver (so its first signature input is the
/// receiver and should not be queued as an interface type of its own).
fn fn_has_self(fn_def: &FnDef) -> bool {
    matches!(
        fn_def.associated_item().map(|item| item.kind),
        Some(rustc_public::ty::AssocKind::Fn { has_self: true, .. })
    )
}

/// The concrete generic arguments of `ty`, taken from its `RigidTy::Adt` variant.
fn concrete_ty_args(ty: Ty) -> Option<GenericArgs> {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(_, args)) if !args.0.is_empty() => Some(args),
        _ => None,
    }
}

/// The concrete arguments to instantiate a method with.
///
/// `partial_args` are the method's own explicitly-enumerated args (from an `emit_items` const;
/// `None` for the ordinary impl walk). Their leading `generics_of(def).parent_count` entries
/// stand for the enclosing impl's parameters, so those are taken from `instance_args` (the
/// concrete `Self`'s arguments) instead, making the result fully concrete. Returns `None` unless
/// the result covers every generic parameter of `def`.
pub(crate) fn combine_method_args(
    def: &FnDef,
    partial_args: Option<&GenericArgs>,
    instance_args: Option<&GenericArgs>,
) -> Option<GenericArgs> {
    let parent_count = def.generics_of().parent_count;
    let mut combined: Vec<GenericArgKind> = Vec::new();
    match (partial_args, instance_args) {
        (Some(partial), _) => {
            // The impl's parameters come from the concrete instance; only the method's *own*
            // args (past `parent_count`) are taken from `partial`.
            if let Some(instance) = instance_args {
                combined.extend(
                    instance.0[..parent_count.min(instance.0.len())]
                        .iter()
                        .cloned(),
                );
            }
            combined.extend(
                partial.0[parent_count.min(partial.0.len())..]
                    .iter()
                    .cloned(),
            );
        }
        (None, Some(instance)) => combined.extend(instance.0.iter().cloned()),
        (None, None) => return None,
    }
    (combined.len() == def.generics_len()).then_some(GenericArgs(combined))
}

impl AutozngContext {
    /// Create a context that carries the driver's [`Config`].
    pub fn new(cfg: Config) -> Self {
        AutozngContext {
            cfg: RefCell::new(cfg),
            data: AutozngContextData::default(),
        }
    }

    /// The driver [`Config`], as adjusted by any `#[autozng::policy]` seen so far.
    pub fn config(&self) -> std::cell::Ref<'_, Config> {
        self.cfg.borrow()
    }

    /// The driver [`Config`], mutably.
    ///
    /// Used by `#[autozng::policy]` (see [`crate::attrs::attributes::policy`]) to adjust the
    /// configuration from the crate root.
    pub fn config_mut(&self) -> std::cell::RefMut<'_, Config> {
        self.cfg.borrow_mut()
    }

    /// All types recorded so far, keyed by their (base) `Ty`.
    ///
    /// Used by the render step to build a [`zngur_def::ZngurSpec`]. The borrow must be
    /// dropped before calling any method that records more types.
    pub fn ty_records(&self) -> std::cell::Ref<'_, FxHashMap<Ty, TyRecord>> {
        self.data.tys.borrow()
    }

    /// The free functions recorded for emission (deduplicated, discovery order).
    pub fn fn_records(&self) -> std::cell::Ref<'_, IndexSet<FnRecord>> {
        self.data.funcs.borrow()
    }

    /// The trait declarations recorded for emission (discovery order).
    ///
    /// Used by the render step to build the `ZngurSpec::traits` mapping. The borrow must be
    /// dropped before calling any method that records more items.
    pub fn trait_records(&self) -> std::cell::Ref<'_, Vec<ExportedTraitRecord>> {
        self.data.traits.borrow()
    }

    /// The `#[autozng::emit_on_implementers]` records (discovery order).
    ///
    /// While rendering a type, each record is checked against the type; when it applies, its
    /// functions are emitted on that type (see `render::build_emit_for_methods`).
    pub fn emit_for_records(&self) -> std::cell::Ref<'_, Vec<EmitForRecord>> {
        self.data.emit_for.borrow()
    }

    /// The export name recorded for `key` (`#[autozng::rename]`), if any, along with the span of
    /// the attribute that produced it.
    pub fn export_name_for<T: IntoMetadataKey + ?Sized>(&self, key: &T) -> Option<ExportName> {
        self.data
            .metadata
            .borrow()
            .get(&key.to_metadata_key())
            .and_then(|record| record.export_name.clone())
    }

    /// Record a `rename(trait = "...")` spec on the ADT whose trait methods it renames.
    pub fn add_trait_rename(&self, adt: &AdtDef, spec: TraitRenameSpec) {
        self.data
            .trait_renames
            .borrow_mut()
            .entry(adt.def_id())
            .or_default()
            .push(spec);
    }

    /// The `rename(trait = "...")` specs recorded on `adt`, in attribute order.
    pub fn trait_renames_for_adt(&self, adt: &AdtDef) -> Vec<TraitRenameSpec> {
        self.data
            .trait_renames
            .borrow()
            .get(&adt.def_id())
            .cloned()
            .unwrap_or_default()
    }

    /// Every `rename(trait = "...")` spec, in discovery order.
    pub fn trait_rename_specs(&self) -> Vec<TraitRenameSpec> {
        self.data
            .trait_renames
            .borrow()
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Mark `span`'s `trait_path` spec as having matched an emitted trait method.
    pub fn mark_trait_rename_matched(&self, span: Span) {
        self.data.matched_trait_renames.borrow_mut().insert(span);
    }

    /// Whether `span`'s `trait_path` spec matched an emitted trait method.
    pub fn trait_rename_matched(&self, span: Span) -> bool {
        self.data.matched_trait_renames.borrow().contains(&span)
    }

    /// The `#[autozng::layout(...)]` override recorded for `key`, if any.
    ///
    /// Consulted while rendering a type: when present it replaces the automatic
    /// `StackAllocated` layout derived from the type's real size and alignment.
    pub fn layout_override_for<T: IntoMetadataKey + ?Sized>(
        &self,
        key: &T,
    ) -> Option<LayoutOverride> {
        self.data
            .metadata
            .borrow()
            .get(&key.to_metadata_key())
            .and_then(|record| record.layout.clone())
    }

    /// The `#[autozng::walk_filter(...)]` recorded for `key`, if any.
    ///
    /// Inherited filters are materialised onto the item by `AutozngAttrsExt`, so this is a
    /// plain metadata read.
    pub fn walk_filter_for<T: IntoMetadataKey + ?Sized>(&self, key: &T) -> Option<WalkFilter> {
        self.data
            .metadata
            .borrow()
            .get(&key.to_metadata_key())
            .and_then(|record| {
                record
                    .walk_filter
                    .as_ref()
                    .map(|record| record.filter.clone())
            })
    }

    /// The effective filter for an ADT: its own filter, else `All` for an explicit
    /// `emit`/`emit_items`, else the crate-wide filter.
    pub fn effective_filter_for_adt(&self, adt: &AdtDef) -> WalkFilter {
        self.walk_filter_for(adt).unwrap_or_else(|| {
            let explicit = self
                .data
                .tys
                .borrow()
                .get(&canonical_ty(adt.ty()))
                .is_some_and(|record| record.selected_explicitly);
            if explicit {
                WalkFilter::default()
            } else {
                self.config().walk_filter.clone()
            }
        })
    }

    /// The filter of the walk in progress, or the crate-wide filter.
    pub(crate) fn current_filter(&self) -> WalkFilter {
        self.current_walk_context()
            .map(|walk| walk.filter)
            .unwrap_or_else(|| self.config().walk_filter.clone())
    }

    pub fn with_metadata_mut<T: IntoMetadataKey + ?Sized, R>(
        &self,
        key: &T,
        f: impl FnOnce(MetadataRecordRefMut) -> R,
    ) -> R {
        f(self.data.metadata_mut_by_key(key.to_metadata_key()))
    }

    /// Record a type as selected for emission, remembering the span of the item (or the
    /// `#[autozng::emit]` / `emit_items`) that selected it.
    pub fn record_ty(&self, ty: &Ty, span: Span) {
        self.record_ty_with_reason(ty, span, false);
    }

    /// Like [`AutozngContext::record_ty`], but marks an explicit `emit` / `emit_items`
    /// selection, which overrides the walk filter for the type's subtree.
    pub fn record_ty_with_reason(&self, ty: &Ty, span: Span, explicit: bool) {
        let ty = canonical_ty(*ty);
        if let Some(adt) = ty.adt_def()
            && adt.is_unstable()
        {
            return;
        }
        if self.data.have_ty(&ty) {
            if explicit {
                if let Some(record) = self.data.tys.borrow_mut().get_mut(&ty) {
                    record.selected_explicitly = true;
                }
            }
            self.enqueue_concrete(ty);
            return;
        }
        let mut record = self.data.ty_record_mut(ty, span);
        record.selected_explicitly = explicit;
        self.enqueue_concrete(ty);
    }

    /// Record an ADT as selected for emission, marking an explicit selection.
    pub fn record_adt_with_reason(&self, adt: &AdtDef, span: Span, explicit: bool) {
        if adt.is_unstable() {
            return;
        }
        let ty = canonical_ty(adt.ty());
        if self.data.have_ty(&ty) {
            if explicit {
                if let Some(record) = self.data.tys.borrow_mut().get_mut(&ty) {
                    record.selected_explicitly = true;
                }
            }
            self.enqueue_concrete(ty);
            return;
        }
        let mut record = self.data.ty_record_mut(ty, span);
        record.selected_explicitly = explicit;
        self.enqueue_concrete(ty);
    }

    /// Request that `ty` get its own IDL `type` block, attributed to `reason`.
    ///
    /// The request inherits the current walk's phase/filter and a child budget; at the crate root
    /// it starts from the configured budgets. A type is queued at most once (`walk_states`
    /// covers both recorded types and already-requested ones). A local type that is not visible
    /// from the crate root is **not** queued: the generated code cannot name it, so the field /
    /// function that exposed it is skipped during rendering instead (see `render`). Numeric
    /// primitives, the unit tuple, and types with no IDL representations are likewise never queued.
    ///
    /// At an exhausted budget the type is recorded as an opaque block instead of being queued
    /// (struct/union: layout only; enum: variant names, no fields).
    pub fn request_ty(&self, ty: Ty, reason: VisitReason, requested_as: RequestedAs, span: Span) {
        self.request_ty_with_filter(ty, reason, requested_as, span, self.current_filter());
    }

    /// Like [`AutozngContext::request_ty`], but with an explicit walk filter for the subtree.
    pub fn request_ty_with_filter(
        &self,
        ty: Ty,
        reason: VisitReason,
        requested_as: RequestedAs,
        span: Span,
        filter: WalkFilter,
    ) {
        let ty = canonical_ty(ty);
        if let Some(adt) = ty.adt_def()
            && (matches!(adt.visibility(), Visibility::PrivateAPI) || adt.is_unstable())
        {
            return;
        }
        // Signature types carry late-bound lifetimes (`Iter<'_, T>`); the IDL drops
        // lifetimes, so use one region-erased identity for the block and for dedupe.
        let key = ty;
        if !can_have_type_block(key) {
            return;
        }
        // A trait generic argument only needs a layout stub: its interface is not part of the
        // trait's, and it must not trigger `emit_on_implementers`.
        if reason == VisitReason::AsGenericArg {
            let mut states = self.data.walk_states.borrow_mut();
            if states.contains_key(&key) {
                return;
            }
            states.insert(
                key,
                WalkState::declare(key, reason, filter, WalkParts::empty()),
            );
            drop(states);
            self.record_ty(&key, span);
            return;
        }
        let target = WalkTarget::of(key);
        let child_base = base_ty(key);
        let current = self.current_walk_context();
        let (budget, phase, parent, ancestors) = match &current {
            Some(walk) => {
                let cyclic = walk.ancestors.contains(&child_base);
                let mut ancestors = walk.ancestors.clone();
                if !cyclic {
                    ancestors.push(child_base);
                }
                (
                    walk.budget.child(target, cyclic),
                    walk.phase,
                    Some(walk.owner),
                    ancestors,
                )
            }
            None => {
                let config = self.config();
                (
                    WalkBudget::root(
                        config.max_walk_depth,
                        config.max_external_depth,
                        config.max_self_ref_depth,
                    ),
                    WalkPhase::InterfaceDrain,
                    None,
                    smallvec::smallvec![child_base],
                )
            }
        };
        let request = WalkRequest {
            ty: key,
            key,
            reason,
            phase,
            budget,
            requested_as,
            span,
            parent,
            filter,
            ancestors,
        };

        let mut states = self.data.walk_states.borrow_mut();
        let request_by_value = requested_as == RequestedAs::Value;
        let existing = states.get(&key).map(|state| {
            state.intent == WalkIntent::Declare
                && (reason.precedence() > state.strongest_reason.precedence()
                    || request.budget.remaining_depth > state.best_remaining_depth
                    || request.budget.remaining_external > state.best_remaining_external
                    || (!request.budget.std_hop_used && state.best_std_hop_used))
        });
        if matches!(existing, Some(false)) {
            if request_by_value && let Some(state) = states.get_mut(&key) {
                state.seen_by_value = true;
            }
            return;
        }
        if request.budget.exhausted() {
            let walked = if key.adt_def().is_some_and(|adt| adt.kind().is_enum()) {
                WalkParts::VARIANTS
            } else {
                WalkParts::empty()
            };
            states.insert(
                key,
                WalkState::declare(key, reason, request.filter.clone(), walked),
            );
            drop(states);
            self.record_ty(&key, span);
            return;
        }
        states.insert(key, WalkState::requested(key, &request));
        drop(states);
        self.data.visit_queue.borrow_mut().push_back(request);
    }

    /// A request that starts a walk at `ty` with the configured root budgets.
    pub fn root_request(
        &self,
        ty: Ty,
        reason: VisitReason,
        phase: WalkPhase,
        span: Span,
    ) -> WalkRequest {
        let (max_walk_depth, max_external_depth, max_self_ref_depth, filter) = {
            let config = self.config();
            (
                config.max_walk_depth,
                config.max_external_depth,
                config.max_self_ref_depth,
                config.walk_filter.clone(),
            )
        };
        let ty = canonical_ty(ty);
        WalkRequest {
            ty,
            key: ty,
            reason,
            phase,
            budget: WalkBudget::root(max_walk_depth, max_external_depth, max_self_ref_depth),
            requested_as: RequestedAs::Value,
            span,
            parent: None,
            filter,
            ancestors: smallvec::smallvec![base_ty(ty)],
        }
    }

    /// Push `walk` as the current walk for the duration of `f`.
    pub fn with_walk<R>(&self, walk: WalkContext, f: impl FnOnce() -> R) -> R {
        self.data.current_walk.borrow_mut().push(walk);
        let result = f();
        self.data.current_walk.borrow_mut().pop();
        result
    }

    fn current_walk_context(&self) -> Option<WalkContext> {
        self.data.current_walk.borrow().last().cloned()
    }

    /// The walk state recorded for `ty`, if any.
    pub fn walk_state(&self, ty: &Ty) -> Option<WalkState> {
        self.data
            .walk_states
            .borrow()
            .get(&canonical_ty(*ty))
            .cloned()
    }

    /// Store the effective filter on an already-recorded type.
    pub(crate) fn set_walk_filter(&self, ty: &Ty, filter: WalkFilter) {
        if let Some(state) = self
            .data
            .walk_states
            .borrow_mut()
            .get_mut(&canonical_ty(*ty))
        {
            state.filter = filter;
        }
    }

    /// Pop the next request waiting for a third-pass visit, if any.
    ///
    /// The caller visits it as if it carried an explicit `emit` (see [`crate::analyze`]); that
    /// visit may queue further types. The queue is empty once every interface type has been
    /// recorded.
    pub fn pop_ty_visit(&self) -> Option<WalkRequest> {
        self.data.visit_queue.borrow_mut().pop_front()
    }

    /// Record that the interface of `key` has been visited.
    ///
    /// The state becomes a `Walk`: a later request with a smaller budget must not replace it with
    /// a declaration that drops the interface it already has (see
    /// [`AutozngContext::request_ty_with_filter`]). A walk that used the type by value also
    /// enables its by-value methods and constructor, which a truncated declaration had disabled.
    pub fn mark_walked(&self, key: Ty, requested_as: RequestedAs) {
        if let Some(state) = self.data.walk_states.borrow_mut().get_mut(&key) {
            state.intent = WalkIntent::Walk;
            state.walked = WalkParts::all();
            if requested_as == RequestedAs::Value {
                state.seen_by_value = true;
            }
        }
    }

    /// Queue every type exposed by `fn_def`'s signature (its inputs and its output) for a
    /// third-pass visit.
    ///
    /// `args` are the function's explicitly-enumerated generic arguments, if any; when they cover
    /// every generic parameter the signature is instantiated first, so a monomorphization queues
    /// its concrete argument types rather than its type parameters.
    fn request_signature_types(
        &self,
        fn_def: FnDef,
        args: Option<&GenericArgs>,
        reason: VisitReason,
        span: Span,
    ) {
        let signature = instantiate_signature(fn_def, args);
        let filter = self
            .walk_filter_for(&fn_def)
            .unwrap_or_else(|| self.current_filter());
        // A method's receiver is its `self` type, which is already part of the interface (the
        // method hangs off it). Queueing `Box<Self>` / `Pin<&Self>` would add orphan types for a
        // receiver that is unsupported (and so skipped) anyway.
        let skip = usize::from(fn_has_self(&fn_def));
        let output = signature.output();
        for ty in signature
            .inputs()
            .iter()
            .skip(skip)
            .copied()
            .chain(std::iter::once(output))
        {
            let requested_as = match ty.kind() {
                TyKind::RigidTy(RigidTy::Ref(_, _, Mutability::Not)) => RequestedAs::Ref,
                TyKind::RigidTy(RigidTy::Ref(_, _, Mutability::Mut)) => RequestedAs::RefMut,
                _ => RequestedAs::Value,
            };
            self.request_ty_with_filter(ty, reason, requested_as, span, filter.clone());
        }
    }

    /// Queue every type exposed by the signature of each of `fns` for a third-pass visit.
    ///
    /// Used by the `emit_on_implementers` records, whose functions are emitted on every matching
    /// type (the concrete `Self` is only known while rendering, but a concrete argument / return
    /// type like `String` is already queueable here).
    fn request_trait_fn_records_types(&self, fns: &IndexSet<TraitFnRecord>) {
        for trait_fn in fns {
            self.request_signature_types(
                trait_fn.def,
                Some(&trait_fn.generics),
                VisitReason::EmitOnImplementers,
                trait_fn.span,
            );
        }
    }

    /// Queue `ty`'s instantiated method signatures for the concrete worklist.
    ///
    /// Called when a concrete instance is recorded; `pop_concrete` drains the worklist.
    pub fn enqueue_concrete(&self, ty: Ty) {
        let ty = canonical_ty(ty);
        let Some(adt) = ty.adt_def() else {
            return;
        };
        if !ty.is_concrete() || concrete_ty_args(ty).is_none() || self.is_declared_ty(&ty) {
            return;
        }
        if self.data.seen_concrete.borrow_mut().insert(ty) {
            self.data.pending_concrete.borrow_mut().push_back((ty, adt));
        }
    }

    /// Pop the next concrete instance whose method signatures need queueing.
    pub fn pop_concrete(&self) -> Option<(Ty, AdtDef)> {
        self.data.pending_concrete.borrow_mut().pop_front()
    }

    /// Queue the concrete signature types of the methods recorded on `adt`'s definition record.
    ///
    /// Methods are recorded on the definition (`Option<T>`), so their uninstantiated signatures
    /// queue `Iter<T>`; the concrete `Iter<char>` used when rendering `Option<char>` must be
    /// queued separately or the IDL references a type block that does not exist.
    pub fn request_concrete_method_signature_types(&self, ty: Ty, adt: &AdtDef) {
        let Some(instance_args) = concrete_ty_args(ty) else {
            return;
        };

        let methods: Vec<(FnDef, Option<GenericArgs>)> = {
            let tys = self.data.tys.borrow();
            tys.get(&canonical_ty(adt.ty()))
                .map(|record| {
                    record
                        .impls
                        .values()
                        .flat_map(|impl_record| impl_record.methods.iter())
                        .map(|method| (method.def, method.partial_args.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };

        let (budget, filter, requested_as, ancestors) = match self.current_walk_context() {
            Some(walk) => (walk.budget, walk.filter, walk.requested_as, walk.ancestors),
            None => {
                let (budget, filter) = match self
                    .walk_state(&ty)
                    .filter(|state| state.best_remaining_depth > 0)
                {
                    Some(state) => {
                        let config = self.config();
                        (
                            WalkBudget {
                                remaining_depth: state.best_remaining_depth,
                                remaining_external: state.best_remaining_external,
                                self_ref_limit: config.max_self_ref_depth,
                                std_hop_used: state.best_std_hop_used,
                            },
                            state.filter,
                        )
                    }
                    None => {
                        let config = self.config();
                        (
                            WalkBudget::root(
                                config.max_walk_depth,
                                config.max_external_depth,
                                config.max_self_ref_depth,
                            ),
                            config.walk_filter.clone(),
                        )
                    }
                };
                (
                    budget,
                    filter,
                    RequestedAs::Value,
                    smallvec::smallvec![base_ty(ty)],
                )
            }
        };
        let walk = WalkContext {
            owner: ty,
            phase: WalkPhase::ConcreteSignature,
            budget,
            filter,
            span: span_of_def(adt.def_id()),
            requested_as,
            ancestors,
        };
        self.with_walk(walk, || {
            for (def, partial_args) in methods {
                let Some(args) =
                    combine_method_args(&def, partial_args.as_ref(), Some(&instance_args))
                else {
                    continue;
                };
                self.request_signature_types(
                    def,
                    Some(&args),
                    VisitReason::ConcreteMethodSignature,
                    span_of_def(def.def_id()),
                );
            }
        });
    }

    /// Whether `ty` was recorded as a declaration-only (truncated) block.
    pub fn is_declared_ty(&self, ty: &Ty) -> bool {
        self.walk_state(ty)
            .is_some_and(|state| state.intent == WalkIntent::Declare)
    }

    /// Queue the concrete type arguments of a trait reference (skipping `Self`) as layout-only
    /// blocks. The trait block names them, but their interface is not part of the trait's.
    fn request_trait_arg_types(&self, trait_ref: &TraitRef, span: Span) {
        let args = trait_ref.args();
        for arg in args.0.iter().skip(1) {
            if let GenericArgKind::Type(ty) = arg {
                self.request_ty(
                    *ty,
                    VisitReason::AsGenericArg,
                    RequestedAs::GenericArg,
                    span,
                );
            }
        }
    }

    /// Whether the walk filter allows this trait method.
    fn trait_method_allowed(&self, trait_def: &TraitDef, fn_def: &FnDef) -> bool {
        let filter = self
            .walk_filter_for(trait_def)
            .unwrap_or_else(|| self.current_filter());
        let name = fn_def
            .name()
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string();
        let trait_path = trait_def.name();
        let item = WalkItem {
            kind: WalkItemKind::TraitMethod,
            name: Some(&name),
            receiver: Some(receiver_kind(fn_def)),
            visibility: Some(visibility_kind(fn_def.visibility())),
            trait_path: Some(trait_path.trim_start_matches("::")),
        };
        filter.allows(&item)
    }

    /// The span that selected `field` on the ADT definition's [`TyRecord`].
    ///
    /// Fields are recorded on the definition (`Option<T>`), not on each concrete instance
    /// (`Option<Ipv4Addr>`). Rendering a concrete instance consults the definition for the
    /// selection, then instantiates the field type with the instance's arguments.
    pub fn field_emit_span(&self, adt: &AdtDef, field: &FieldDef) -> Option<Span> {
        let tys = self.data.tys.borrow();
        let record = tys.get(&canonical_ty(adt.ty()))?;
        record.fields.get(&field.def_id()).map(|field| field.span)
    }

    /// Record a field as selected for emission onto `ty`'s [`TyRecord`].
    ///
    /// Called while walking an ADT's fields in `analyze.rs` (through
    /// [`rustc_public::ty::FieldDef`]'s `EmitTarget` impl), after the `emit`/`skip` action and
    /// the field's visibility have been resolved.
    pub fn record_field(&self, ty: Ty, field: FieldDef, span: Span) {
        if field.is_unstable() {
            return;
        }
        let filter = self.current_filter();
        let name = field.name.to_string();
        let item = WalkItem {
            kind: WalkItemKind::Field,
            name: Some(&name),
            receiver: None,
            visibility: Some(visibility_kind(field.visibility())),
            trait_path: None,
        };
        if !filter.allows(&item) {
            return;
        }
        // A field's type is part of the type's interface: it must get its own `type` block.
        // A `walk_filter` on the field replaces the filter for the field type's subtree.
        let child_filter = self.walk_filter_for(&field).unwrap_or(filter);
        // A field of a by-reference-only type is itself only reachable by reference.
        let requested_as = match self.current_walk_context() {
            Some(walk) if walk.requested_as != RequestedAs::Value => RequestedAs::Ref,
            _ => RequestedAs::Value,
        };
        self.request_ty_with_filter(
            field.ty(),
            VisitReason::Field,
            requested_as,
            span,
            child_filter,
        );
        let mut record = self.data.ty_record_mut(ty, span);
        record
            .fields
            .insert(field.def_id(), FieldRecord { def: field, span });
    }

    /// Whether a type has already been recorded (i.e. selected for emission). Used to gate
    /// per-type work (like walking impl blocks) so it never implicitly emits a type that the
    /// `emit`/`skip` attributes left out.
    pub fn is_ty_recorded(&self, ty: &Ty) -> bool {
        self.data.have_ty(ty)
    }

    /// Record a free function for emission (as a `fn` entry in the IDL spec).
    ///
    /// `partial_args` are the function's own explicitly-enumerated generic args, if any (see
    /// [`FnRecord::partial_args`]); the ordinary crate walk passes `None`. `span` is the span of
    /// the item that selected the function.
    pub fn record_fn(&self, fn_def: FnDef, partial_args: Option<GenericArgs>, span: Span) {
        if fn_def.is_unstable() {
            return;
        }
        // Every type the function exposes is part of the interface: queue it for a third-pass
        // visit so it gets its own `type` block.
        self.request_signature_types(
            fn_def,
            partial_args.as_ref(),
            VisitReason::MethodSignature,
            span,
        );
        self.data.funcs.borrow_mut().insert(FnRecord {
            def: fn_def,
            clauses: fn_def.clauses_of(),
            partial_args,
            span,
        });
    }

    /// Record a concrete associated fn as belonging to `ty`.
    ///
    /// Called while walking an ADT's inherent impl blocks (`analyze.rs`); the `clauses` on the
    /// inserted [`ImplRecord`] are the impl's clauses. `partial_args` is the method's own
    /// explicitly-enumerated generic args, if any (see [`FnRecord::partial_args`]); the plain
    /// impl walk passes `None`. `span` is the span of the item that selected the method.
    pub fn record_impl_method(
        &self,
        ty: Ty,
        impl_def: ImplDef,
        fn_def: FnDef,
        partial_args: Option<GenericArgs>,
        span: Span,
    ) {
        if fn_def.is_unstable() {
            return;
        }
        let filter = self.current_filter();
        let name = fn_def
            .name()
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string();
        let item = WalkItem {
            kind: WalkItemKind::InherentMethod,
            name: Some(&name),
            receiver: Some(receiver_kind(&fn_def)),
            visibility: Some(visibility_kind(fn_def.visibility())),
            trait_path: None,
        };
        if !filter.allows(&item) {
            return;
        }
        // Every type the method exposes is part of the interface: queue it for a third-pass visit.
        self.request_signature_types(
            fn_def,
            partial_args.as_ref(),
            VisitReason::MethodSignature,
            span,
        );
        let mut record = self.data.ty_record_mut(ty, span);
        let impl_record = record
            .impls
            .entry(impl_def.def_id())
            .or_insert_with(|| ImplRecord {
                def_id: impl_def.def_id(),
                methods: IndexSet::default(),
                clauses: impl_def.clauses_of(), // TODO clauses of
            });
        impl_record.methods.insert(FnRecord {
            def: fn_def,
            clauses: fn_def.clauses_of(),
            partial_args,
            span,
        });
    }

    /// Record a trait method onto the implementing type's [`TyRecord`].
    ///
    /// Called when an `emit_items` const names a trait method (see [`TraitFnRecord`]); the
    /// implementing type is the trait reference's self type, and `span` is the span of the item
    /// that selected it.
    pub fn record_trait_type_fn(
        &self,
        def: FnDef,
        trait_ref: TraitRef,
        generics: GenericArgs,
        span: Span,
    ) {
        if !self.trait_method_allowed(&trait_ref.def_id, &def) {
            return;
        }
        // Every type the trait method exposes is part of the interface.
        self.request_trait_arg_types(&trait_ref, span);
        self.request_signature_types(def, Some(&generics), VisitReason::TraitMethod, span);
        let ty = canonical_ty(trait_ref.self_ty());
        let mut record = self.data.ty_record_mut(ty, span);
        record.trait_fns.insert(TraitFnRecord {
            def,
            clauses: def.clauses_of(),
            trait_def: trait_ref.def_id,
            trait_ref: Some(trait_ref),
            generics,
            span,
        });
    }

    /// Whether a trait declaration has already been recorded (i.e. selected for emission). Used
    /// to gate walking its associated functions so a `skip`-ed trait never leaks in.
    pub fn is_trait_recorded(&self, trait_def: &TraitDef) -> bool {
        self.data
            .traits
            .borrow()
            .iter()
            .any(|record| record.decl.def_id == *trait_def)
    }

    /// Whether the trait was recorded as a *concrete* instantiation (rather than a declaration).
    ///
    /// A concrete record already carries its associated functions (with the concrete arguments),
    /// so the declaration walk in `CrateAnalysisVisitor::visit_trait` must not record them again
    /// with their identity signatures.
    pub fn is_trait_concretely_recorded(&self, trait_def: &TraitDef) -> bool {
        self.data
            .traits
            .borrow()
            .iter()
            .any(|record| record.decl.def_id == *trait_def && record.concrete.is_some())
    }

    /// Reject a trait that cannot be exported as an IDL `trait` block.
    ///
    /// A `trait` block exists so a C++ type can inherit the trait and be used through `dyn`
    /// (see the IDL `trait` block docs), and that requires dyn compatibility: no generic methods,
    /// no associated constants, and so on. A trait that is not dyn compatible cannot cross the
    /// ABI as a trait object, so selecting one is a hard error rather than a warn-and-skip. The
    /// `Fn`/`FnMut`/`FnOnce` call traits are likewise never a `trait` block: they are represented
    /// by the `dyn Fn(...)` type (zngur builds a `std::function`-style bridge), so selecting one
    /// is a warning.
    ///
    /// `span` is the item that selected the trait; the error points at the declaration itself and
    /// only adds a `selected here` note when the two differ. Returns `false` when the trait was
    /// rejected and must not be recorded. The diagnosis is emitted once per trait.
    fn ensure_trait_exportable(&self, def: TraitDef, span: Span) -> bool {
        with_ctxt(|ctx| {
            let fn_trait = fn_trait_name(&visible_path(&def));
            let dyn_compatible = ctx.tcx.is_dyn_compatible(def.def_id().internal(ctx.tcx));
            if dyn_compatible && fn_trait.is_none() {
                if self.data.unexportable_traits.borrow().contains(&def) {
                    return false;
                }
                let static_method = def.associated_items().into_iter().find(|item| {
                    matches!(
                        &item.kind,
                        rustc_public::ty::AssocKind::Fn {
                            has_self: false,
                            ..
                        }
                    )
                });
                if let Some(item) = static_method {
                    self.data.unexportable_traits.borrow_mut().push(def);
                    let name = match &item.kind {
                        rustc_public::ty::AssocKind::Fn { name, .. } => name.clone(),
                        _ => unreachable!(),
                    };
                    let name = name.rsplit("::").next().unwrap_or_default();
                    let source_span = span_of_def(def.def_id());
                    let method_span = span_of_def(item.def_id.def_id());
                    let mut diag = ctx.tcx.dcx().struct_span_err(
                        source_span,
                        format!(
                            "autozng: trait `{}` has a static method `{name}` and cannot be exported as a `trait` block",
                            def.name()
                        ),
                    );
                    diag.span_note(method_span, "static method declared here");
                    if !span.is_dummy() && !source_span.contains(span) {
                        diag.span_note(span, "selected here");
                    }
                    diag.emit();
                    return false;
                }
                return true;
            }
            if self.data.unexportable_traits.borrow().contains(&def) {
                return false;
            }
            self.data.unexportable_traits.borrow_mut().push(def);

            if let Some(name) = fn_trait {
                // A call trait has no meaningful vtable export; selecting one is a warning and
                // the trait is skipped.
                emit_warning_for_span(
                    span,
                    format!(
                        "autozng: `{name}` cannot be exported as a `trait` block; use the `dyn` type instead"
                    ),
                );
                return false;
            }

            let source_span = span_of_def(def.def_id());
            let mut diag = ctx.tcx.dcx().struct_span_err(
                source_span,
                format!(
                    "autozng: trait `{}` is not dyn compatible and cannot be exported as a `trait` block",
                    def.name()
                ),
            );
            if !span.is_dummy() && !source_span.contains(span) {
                diag.span_note(span, "selected here");
            }
            diag.emit();
            false
        })
    }

    /// Reject a generic trait selected by `#[autozng::emit]`.
    ///
    /// An IDL `trait` block describes a concrete dyn compatible trait; it cannot define type or
    /// const parameters. A declaration has no concrete `Self: Trait<Args>` reference for the
    /// bridge, so only a concrete dyn compatible trait can be exported.
    /// `emit_items(emit_traits = true)` names those instantiations (see
    /// [`AutozngContext::record_concrete_trait`]).
    ///
    /// Returns `false` when the trait was rejected and must not be recorded.
    fn ensure_trait_declaration_exportable(
        &self,
        def: TraitDef,
        decl: &TraitDecl,
        span: Span,
    ) -> bool {
        // Lifetimes do not block export: they are only lost when a signature is converted to an
        // IDL type.
        let generics = decl.generics_of();
        let generic = generics.params.iter().any(|param| {
            !(generics.has_self && param.index == 0)
                && !matches!(&param.kind, GenericParamDefKind::Lifetime)
        });
        if !generic {
            return true;
        }
        with_ctxt(|ctx| {
            let source_span = span_of_def(def.def_id());
            let name = def.name();
            let name = name.rsplit("::").next().unwrap_or_default();
            let mut diag = ctx.tcx.dcx().struct_span_err(
                source_span,
                format!("autozng: generic trait `{name}` cannot be exported as a `trait` block"),
            );
            if !span.is_dummy() && !source_span.contains(span) {
                diag.span_note(span, "selected here");
            }
            diag.note(
                "name a concrete instantiation with `#[autozng::emit_items(emit_traits = true)]`",
            );
            diag.emit();
        });
        false
    }

    /// Record a *concrete* trait instantiation (`dyn Trait<ConcreteArgs>`) selected by
    /// `#[autozng::emit_items(emit_traits = true)]`.
    ///
    /// Unlike [`AutozngContext::record_trait`] (a declaration, whose methods keep their identity
    /// signature), the concrete `Self: Trait<...>` reference is remembered and each associated
    /// function is recorded with the trait reference's concrete arguments, so `render` can emit
    /// the trait as e.g. `crate::GenericTrait<u32>` with instantiated method signatures. Recording
    /// is idempotent per trait.
    pub fn record_concrete_trait(&self, trait_ref: TraitRef, span: Span) {
        let def = trait_ref.def_id;
        if def.is_unstable() {
            return;
        }
        // Deduplicate per instantiation: the same trait may be enumerated with different concrete
        // arguments (e.g. `GenericTrait<u32>` and `GenericTrait<u8>`), each becoming its own entry.
        if self
            .data
            .traits
            .borrow()
            .iter()
            .any(|record| record.concrete.as_ref() == Some(&trait_ref))
        {
            return;
        }

        // An IDL `trait` block is only usable through `dyn`, so the trait must be dyn compatible
        // (see `ensure_trait_exportable`). This is unreachable through `dyn` syntax — rustc
        // already rejects a `dyn` object of a non-dyn-compatible trait — but keeps the invariant
        // local to the recording entry point.
        if !self.ensure_trait_exportable(def, span) {
            return;
        }

        let decl = with_ctxt(|ctx| stable_trait(def.internal(ctx.tcx)));

        // A concrete instantiation records every associated function with the trait reference's
        // concrete arguments (`Self` first), so the method signatures can be instantiated. The
        // functions are built directly rather than through `record_trait_method` so they land on
        // *this* instantiation's record and not on the first record for the same trait.
        let generics = trait_ref.args().clone();
        self.request_trait_arg_types(&trait_ref, span);
        let mut fns = IndexSet::default();
        for assoc_item in def.associated_items() {
            if let Some((fn_def, _identity_generics)) = assoc_item.opt_fn_def() {
                // The instantiation's arguments are concrete, so the method's interface types are
                // queued as concrete types.
                self.request_signature_types(fn_def, Some(&generics), VisitReason::EmitTrait, span);
                fns.insert(TraitFnRecord {
                    def: fn_def,
                    trait_def: def,
                    trait_ref: Some(trait_ref.clone()),
                    generics: generics.clone(),
                    clauses: fn_def.clauses_of(),
                    span,
                });
            }
        }

        self.data.traits.borrow_mut().push(ExportedTraitRecord {
            decl,
            span,
            source_span: span_of_def(def.def_id()),
            clauses: def.clauses_of(),
            fns,
            concrete: Some(trait_ref),
        });
    }

    /// Record a `#[autozng::emit_on_implementers]` trait declaration.
    ///
    /// The trait's own clauses are remembered; while rendering a type, the trait is checked
    /// against it and its functions are emitted when it applies (see [`EmitForRecord`]).
    /// Recording is idempotent per trait declaration.
    pub fn record_emit_for_trait(&self, trait_def: TraitDef, span: Span) {
        if trait_def.is_unstable() {
            return;
        }
        if self
            .data
            .emit_for
            .borrow()
            .iter()
            .any(|record| record.trait_def == trait_def && record.impl_def.is_none())
        {
            return;
        }
        let fns = trait_fn_records(trait_def, span);
        self.request_trait_fn_records_types(&fns);
        self.data.emit_for.borrow_mut().push(EmitForRecord {
            trait_def,
            impl_def: None,
            clauses: trait_def.clauses_of(),
            span,
            fns,
        });
    }

    /// Record a `#[autozng::emit_on_implementers]` trait impl block.
    ///
    /// The impl block's clauses are remembered (the trait's *declaration* functions are stored,
    /// since the impl only supplies the clauses and a matching `Self` type). While rendering a
    /// type, the impl is matched against it and, when it applies, the trait's functions are
    /// emitted on that type. Recording is idempotent per impl block.
    pub fn record_emit_for_impl(&self, impl_def: ImplDef, trait_ref: TraitRef, span: Span) {
        if trait_ref.def_id.is_unstable() {
            return;
        }
        if self
            .data
            .emit_for
            .borrow()
            .iter()
            .any(|record| record.impl_def.map(|def| def.def_id()) == Some(impl_def.def_id()))
        {
            return;
        }
        let trait_def = trait_ref.def_id;
        self.request_trait_arg_types(&trait_ref, span);
        let fns = trait_fn_records(trait_def, span);
        self.request_trait_fn_records_types(&fns);
        self.data.emit_for.borrow_mut().push(EmitForRecord {
            trait_def,
            impl_def: Some(impl_def),
            clauses: impl_def.clauses_of(),
            span,
            fns,
        });
    }

    /// Record a trait declaration selected for emission (see [`ExportedTraitRecord`]).
    ///
    /// `span` is the span of the `#[autozng::emit]` that selected the trait; the declaration's
    /// own source span is recorded alongside it. The associated functions are added separately by
    /// [`AutozngContext::record_trait_method`]. Recording is idempotent.
    pub fn record_trait(&self, def: TraitDef, span: Span) {
        if def.is_unstable() {
            return;
        }
        // An IDL `trait` block is only usable through `dyn`; a non-dyn-compatible trait cannot
        // be exported (see `ensure_trait_exportable`).
        if !self.ensure_trait_exportable(def, span) {
            return;
        }
        // The public `TraitDecl` is the stable description of the trait (`stable_trait` bridges
        // the internal `rustc_middle` definition to it).
        let decl = with_ctxt(|ctx| stable_trait(def.internal(ctx.tcx)));
        // An IDL `trait` block describes a concrete dyn compatible trait, so only a concrete
        // dyn compatible trait can be exported.
        if !self.ensure_trait_declaration_exportable(def, &decl, span) {
            return;
        }
        if self.is_trait_recorded(&def) {
            return;
        }
        self.data.traits.borrow_mut().push(ExportedTraitRecord {
            decl,
            span,
            source_span: span_of_def(def.def_id()),
            clauses: def.clauses_of(),
            fns: IndexSet::default(),
            concrete: None,
        });
    }

    /// Record one of an exported trait's associated functions onto its
    /// [`ExportedTraitRecord`].
    ///
    /// The function's identity args are recorded (there is no concrete `Self` for a
    /// declaration), together with its `Self: Trait<...>` reference. Does nothing if the trait
    /// was not recorded.
    pub fn record_trait_method(
        &self,
        trait_def: TraitDef,
        trait_ref: Option<TraitRef>,
        fn_def: FnDef,
        generics: GenericArgs,
        span: Span,
    ) {
        if fn_def.is_unstable() {
            return;
        }
        if !self.trait_method_allowed(&trait_def, &fn_def) {
            return;
        }
        if let Some(trait_ref) = &trait_ref {
            self.request_trait_arg_types(trait_ref, span);
        }
        // Every type the trait method exposes is part of the interface.
        self.request_signature_types(fn_def, Some(&generics), VisitReason::TraitMethod, span);
        let mut traits = self.data.traits.borrow_mut();
        let Some(record) = traits
            .iter_mut()
            .find(|record| record.decl.def_id == trait_def)
        else {
            return;
        };
        record.fns.insert(TraitFnRecord {
            def: fn_def,
            trait_def,
            trait_ref,
            generics,
            clauses: fn_def.clauses_of(),
            span,
        });
    }
}
