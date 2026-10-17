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

use rustc_hash::FxHashMap;
use rustc_public::CrateDef;
use rustc_public::abi::Layout;
use rustc_public::ty::{AdtDef, FieldDef, FnDef, GenericArgs, ImplDef, TraitRef};
use rustc_public::{DefId, ty::Ty};
use rustc_span::Span;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use crate::config::Config;

use crate::driver::with_ctxt;
use crate::metadata::{
    ExportName, IntoMetadataKey, MetadataKey, MetadataMap, MetadataRecordRefMut,
};
use crate::rustc_ext::{ClausesOfExt, GenericClauses, RustcPublicTyExt};

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
/// Recorded when an `emit_items` const names a trait method (`let _ = Example::to_string;`,
/// `<Example as YarrExt>::yarrrr`), which the impl walk cannot see because trait visiting is
/// stubbed. Emitted as a method of the self type, with the trait as the `use_path`
/// (`fn to_string(&self) -> ::std::string::String use ::std::string::ToString;`), and only if
/// the trait's clauses hold for the concrete arguments.
#[derive(Debug)]
pub struct TraitFnRecord {
    /// The trait method itself (e.g. `ToString::to_string`).
    pub def: FnDef,
    /// `Self: Trait<...>` for the implementing type (e.g. `Example: ToString`).
    pub tref: TraitRef,
    /// The method's generic args, as named by the `let` binding's `FnDef` type (Self first).
    pub generics: GenericArgs,
    /// The trait method's own clauses.
    pub clauses: GenericClauses,
    /// Span of the `#[autozng::emit_items]` that named this method (see [`TyRecord::span`]).
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
    pub def: FieldDef,
    pub span: Span,
}

#[derive(Debug, Default)]
struct AutozngContextData {
    tys: RefCell<FxHashMap<Ty, TyRecord>>,
    metadata: RefCell<MetadataMap>,
    /// Free functions selected for emission, in discovery order.
    funcs: RefCell<IndexSet<FnRecord>>,
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

    fn ty_record_mut(&self, ty: Ty, span: Span) -> std::cell::RefMut<'_, TyRecord> {
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
    cfg: Config,
    data: AutozngContextData,
}

impl AutozngContext {
    /// Create a context that carries the driver's [`Config`].
    pub fn new(cfg: Config) -> Self {
        AutozngContext {
            cfg,
            data: AutozngContextData::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
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

    /// The export name recorded for `key` (`#[autozng::rename]`), if any, along with the span of
    /// the attribute that produced it.
    pub fn export_name_for<T: IntoMetadataKey + ?Sized>(&self, key: &T) -> Option<ExportName> {
        self.data
            .metadata
            .borrow()
            .get(&key.to_metadata_key())
            .and_then(|record| record.export_name.clone())
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

    /// Record a field as selected for emission onto `ty`'s [`TyRecord`].
    ///
    /// Called while walking an ADT's fields in `analyze.rs` (through
    /// [`rustc_public::ty::FieldDef`]'s `EmitTarget` impl), after the `emit`/`skip` action and
    /// the field's visibility have been resolved.
    pub fn record_field(&self, ty: Ty, field: FieldDef, span: Span) {
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
    pub fn record_trait_fn(&self, def: FnDef, tref: TraitRef, generics: GenericArgs, span: Span) {
        let ty = tref.self_ty().base_ty();
        let mut record = self.data.ty_record_mut(ty, span);
        record.trait_fns.insert(TraitFnRecord {
            def,
            clauses: def.clauses_of(),
            tref,
            generics,
            span,
        });
    }
}
