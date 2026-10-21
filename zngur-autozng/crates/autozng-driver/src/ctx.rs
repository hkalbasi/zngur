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
use rustc_public::ty::{
    AdtDef, FieldDef, FnDef, FnSig, GenericArgs, ImplDef, TraitDecl, TraitDef, TraitRef,
};
use rustc_public::{DefId, ty::Ty};
use rustc_span::Span;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::config::Config;

use crate::driver::with_ctxt;
use crate::error::span_of_def;
use crate::metadata::{
    ExportName, IntoMetadataKey, LayoutOverride, MetadataKey, MetadataMap, MetadataRecordRefMut,
};
use crate::rustc_ext::{
    ClausesOfExt, FnDefExt, GenericClauses, GetAssocInfoExt, RustcInternalEx, RustcPublicTyExt,
    Visibility, VisibilityExt, stable_trait,
};
use crate::type_map::{is_type_block_less, map_rustc_public_ty_to_zngur};

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
    /// `&dyn GenericTrait<u32>`. `None` for a declaration selected by `#[autozng::emit]`, which
    /// renders with its own type parameters as `.zng` type variables.
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
    visit_queue: RefCell<VecDeque<Ty>>,
    /// Every type already recorded, or already requested for a third-pass visit.
    ///
    /// Guards [`AutozngContextData::visit_queue`]: a type is queued at most once, so a cyclic
    /// interface (or a type reached through several paths) is never visited twice.
    seen_tys: RefCell<FxHashSet<Ty>>,
}

impl AutozngContextData {
    fn metadata_mut_by_key(&self, key: MetadataKey) -> MetadataRecordRefMut<'_> {
        std::cell::RefMut::map(self.metadata.borrow_mut(), |metadata| {
            metadata.entry(key).or_default()
        })
    }

    fn have_ty(&self, ty: &Ty) -> bool {
        self.tys.borrow().contains_key(ty)
    }

    /// Mark `ty` as seen, so it is never queued for a third-pass visit.
    fn mark_seen(&self, ty: Ty) {
        self.seen_tys.borrow_mut().insert(ty.base_ty());
    }

    fn ty_record_mut(&self, ty: Ty, span: Span) -> std::cell::RefMut<'_, TyRecord> {
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
            })
        })
    }
}

#[derive(Debug, Default)]
pub struct AutozngContext {
    cfg: RefCell<Config>,
    data: AutozngContextData,
}

/// Whether `ty` can be declared as its own `.zng` `type` block.
///
/// A numeric primitive is translated by zngur to a native C++ numeric type and the unit tuple
/// (`()`) is specialised, so neither is ever declared; a type with no `.zng` representation at
/// all (an array, a fn pointer, a type parameter, ...) has no block either. A type with escaping
/// bound vars (e.g. a late-bound lifetime lifted out of a method signature) cannot be named on
/// its own at all. Such types remain usable in method / function signatures — they are simply not
/// queued for a visit.
fn can_have_type_block(ty: Ty) -> bool {
    with_ctxt(|driver| {
        use rustc_middle::ty::TypeVisitableExt;

        if ty.internal(driver.tcx).has_escaping_bound_vars() {
            return false;
        }
        match map_rustc_public_ty_to_zngur(ty) {
            Ok(rust_ty) => !is_type_block_less(&rust_ty),
            Err(_) => false,
        }
    })
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
        let ty = ty.base_ty();
        if !self.data.have_ty(&ty) {
            // insert a default record
            let _ = self.data.ty_record_mut(ty, span);
        };
    }

    /// Record an ADT as selected for emission, remembering the span that selected it.
    pub fn record_adt(&self, adt: &AdtDef, span: Span) {
        let ty = adt.ty().base_ty();
        if !self.data.have_ty(&ty) {
            // insert a default record
            let _ = self.data.ty_record_mut(ty, span);
        }
    }

    /// Request that `ty` get its own `.zng` `type` block by queueing it for the third analysis
    /// pass (see [`crate::analyze`]).
    ///
    /// Called for every type an emitted interface exposes: a field's type, or the inputs and
    /// output of a function / method signature. The type is treated as if it carried an explicit
    /// `#[autozng::emit]` when it is visited, so a type reached only through an interface still
    /// gets a `type` block.
    ///
    /// A type is queued at most once (`seen_tys` covers both recorded types and already-requested
    /// ones). A local type that is not visible from the crate root is **not** queued: the
    /// generated code cannot name it, so the field / function that exposed it is skipped during
    /// rendering instead (see `render`). Numeric primitives and the unit tuple (translated /
    /// specialised by zngur) and types with no `.zng` spelling are likewise never queued.
    pub fn request_ty_visit(&self, ty: Ty) {
        let ty = ty.base_ty();
        if let Some(adt) = ty.adt_def()
            && matches!(adt.visibility(), Visibility::PrivateAPI)
        {
            return;
        }
        if !can_have_type_block(ty) {
            return;
        }
        // `insert` returns `false` when the type was already recorded or requested.
        if self.data.seen_tys.borrow_mut().insert(ty) {
            self.data.visit_queue.borrow_mut().push_back(ty);
        }
    }

    /// Pop the next type waiting for a third-pass visit, if any.
    ///
    /// The caller visits it as if it carried an explicit `emit` (see [`crate::analyze`]); that
    /// visit may queue further types. The queue is empty once every interface type has been
    /// recorded.
    pub fn pop_ty_visit(&self) -> Option<Ty> {
        self.data.visit_queue.borrow_mut().pop_front()
    }

    /// Queue every type exposed by `fn_def`'s signature (its inputs and its output) for a
    /// third-pass visit.
    ///
    /// `args` are the function's explicitly-enumerated generic arguments, if any; when they cover
    /// every generic parameter the signature is instantiated first, so a monomorphization queues
    /// its concrete argument types rather than its type parameters.
    fn request_signature_types(&self, fn_def: FnDef, args: Option<&GenericArgs>) {
        let signature = instantiate_signature(fn_def, args);
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
            self.request_ty_visit(ty);
        }
    }

    /// Queue every type exposed by the signature of each of `fns` for a third-pass visit.
    ///
    /// Used by the `emit_on_implementers` records, whose functions are emitted on every matching
    /// type (the concrete `Self` is only known while rendering, but a concrete argument / return
    /// type like `String` is already queueable here).
    fn request_trait_fn_records_types(&self, fns: &IndexSet<TraitFnRecord>) {
        for trait_fn in fns {
            self.request_signature_types(trait_fn.def, Some(&trait_fn.generics));
        }
    }

    /// Record a field as selected for emission onto `ty`'s [`TyRecord`].
    ///
    /// Called while walking an ADT's fields in `analyze.rs` (through
    /// [`rustc_public::ty::FieldDef`]'s `EmitTarget` impl), after the `emit`/`skip` action and
    /// the field's visibility have been resolved.
    pub fn record_field(&self, ty: Ty, field: FieldDef, span: Span) {
        // A field's type is part of the type's interface: it must get its own `type` block.
        self.request_ty_visit(field.ty());
        let mut record = self.data.ty_record_mut(ty, span);
        record
            .fields
            .insert(field.def_id(), FieldRecord { def: field, span });
    }

    /// Whether a type has already been recorded (i.e. selected for emission). Used to gate
    /// per-type work (like walking impl blocks) so it never implicitly emits a type that the
    /// `emit`/`skip` attributes left out.
    pub fn is_ty_recorded(&self, ty: &Ty) -> bool {
        self.data.have_ty(&ty.base_ty())
    }

    /// Record a free function for emission (as a `fn` entry in the `.zng` spec).
    ///
    /// `partial_args` are the function's own explicitly-enumerated generic args, if any (see
    /// [`FnRecord::partial_args`]); the ordinary crate walk passes `None`. `span` is the span of
    /// the item that selected the function.
    pub fn record_fn(&self, fn_def: FnDef, partial_args: Option<GenericArgs>, span: Span) {
        // Every type the function exposes is part of the interface: queue it for a third-pass
        // visit so it gets its own `type` block.
        self.request_signature_types(fn_def, partial_args.as_ref());
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
        // Every type the method exposes is part of the interface: queue it for a third-pass visit.
        self.request_signature_types(fn_def, partial_args.as_ref());
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
        // Every type the trait method exposes is part of the interface.
        self.request_signature_types(def, Some(&generics));
        let ty = trait_ref.self_ty().base_ty();
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

        let decl = with_ctxt(|ctx| stable_trait(def.internal(ctx.tcx)));

        // A concrete instantiation records every associated function with the trait reference's
        // concrete arguments (`Self` first), so the method signatures can be instantiated. The
        // functions are built directly rather than through `record_trait_method` so they land on
        // *this* instantiation's record and not on the first record for the same trait.
        let generics = trait_ref.args().clone();
        let mut fns = IndexSet::default();
        for assoc_item in def.associated_items() {
            if let Some((fn_def, _identity_generics)) = assoc_item.opt_fn_def() {
                // The instantiation's arguments are concrete, so the method's interface types are
                // queued as concrete types.
                self.request_signature_types(fn_def, Some(&generics));
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

    /// Record a trait declaration as selected for emission (see [`ExportedTraitRecord`]).
    ///
    /// `span` is the span of the `#[autozng::emit]` that selected the trait; the declaration's
    /// own source span is recorded alongside it. The associated functions are added separately by
    /// [`AutozngContext::record_trait_method`]. Recording is idempotent.
    pub fn record_trait(&self, def: TraitDef, span: Span) {
        if self.is_trait_recorded(&def) {
            return;
        }
        // The public `TraitDecl` is the stable description of the trait (`stable_trait` bridges
        // the internal `rustc_middle` definition to it).
        let decl = with_ctxt(|ctx| stable_trait(def.internal(ctx.tcx)));
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
        // Every type the trait method exposes is part of the interface.
        self.request_signature_types(fn_def, Some(&generics));
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
