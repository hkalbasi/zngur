//! The accumulation target for the analysis passes.
//!
//! [`AutozngContext`] collects one [`TyRecord`] per visited type (keyed by `Ty`), the set of
//! free functions selected for emission, plus a [`MetadataMap`] of per-item tool attributes
//! keyed by `MetadataKey`. Both passes in `analyze.rs` (the HIR pass and the
//! `rustc_public` pass) write into the same context.
//!
//! Methods reach a type through [`AutozngContext::record_impl_method`] (called while walking
//! an ADT's inherent impl blocks in `analyze.rs`); free functions through
//! [`AutozngContext::record_fn`].

use indexmap::{IndexMap, IndexSet};

use rustc_hash::FxHashMap;
use rustc_public::CrateDef;
use rustc_public::abi::Layout;
use rustc_public::ty::{AdtDef, FnDef, GenericArgs, ImplDef};
use rustc_public::{DefId, ty::Ty};

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use crate::config::Config;

use crate::driver::with_ctxt;
use crate::metadata::{
    IntoMetadataKey, MetadataKey, MetadataMap, MetadataRecord, MetadataRecordRefMut,
};
use crate::rustc_ext::{ClausesOfExt, GenericClauses, RustcPublicTyExt};

#[derive(Debug, PartialEq, Eq)]
pub struct FnRecord {
    pub def: FnDef,
    pub clauses: GenericClauses,
    pub partial_args: Option<GenericArgs>,
}

impl Hash for FnRecord {
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
    /// Keyed by the impl's `DefId`; populated by [`AutozngContext::record_impl_method`].
    pub impls: IndexMap<DefId, ImplRecord>,
}

#[derive(Debug, Default)]
struct AutozngContextData {
    tys: RefCell<FxHashMap<Ty, TyRecord>>,
    metadata: RefCell<MetadataMap>,
    /// Free functions selected for emission, in discovery order.
    funcs: RefCell<IndexSet<FnDef>>,
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

    fn ty_record_mut(&self, ty: Ty) -> std::cell::RefMut<'_, TyRecord> {
        let adt = ty.adt_def();
        std::cell::RefMut::map(self.tys.borrow_mut(), |tys| {
            tys.entry(ty).or_insert_with(|| TyRecord {
                ty,
                adt,
                layout: ty.layout().ok(),
                clauses: adt.map_or_default(|adt| adt.clauses_of()),
                is_concrete: ty.is_concrete(),
                impls: IndexMap::default(),
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
    pub fn fn_records(&self) -> std::cell::Ref<'_, IndexSet<FnDef>> {
        self.data.funcs.borrow()
    }

    /// The export name recorded for `key` (`#[autozng::rename]`), if any.
    pub fn export_name_for<T: IntoMetadataKey + ?Sized>(&self, key: &T) -> Option<String> {
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

    pub fn record_ty(&self, ty: &Ty) {
        let ty = ty.base_ty();
        if !self.data.have_ty(&ty) {
            // insert a default record
            let _ = self.data.ty_record_mut(ty);
        };
    }

    pub fn record_adt(&self, adt: &AdtDef) {
        let ty = adt.ty().base_ty();
        if !self.data.have_ty(&ty) {
            // insert a default record
            let _ = self.data.ty_record_mut(ty);
        }
    }

    /// Whether a type has already been recorded (i.e. selected for emission). Used to gate
    /// per-type work (like walking impl blocks) so it never implicitly emits a type that the
    /// `emit`/`skip` attributes left out.
    pub fn is_ty_recorded(&self, ty: &Ty) -> bool {
        self.data.have_ty(&ty.base_ty())
    }

    /// Record a free function for emission (as a `fn` entry in the `.zng` spec).
    pub fn record_fn(&self, fn_def: FnDef) {
        self.data.funcs.borrow_mut().insert(fn_def);
    }

    /// Record a concrete associated fn as belonging to `ty`.
    ///
    /// Called while walking an ADT's inherent impl blocks (`analyze.rs`). `partial_args` is
    /// the partially-applied generic args for methods on generic types; the `clauses` on the
    /// inserted [`ImplRecord`] are the impl's clauses.
    pub fn record_impl_method(
        &self,
        ty: Ty,
        impl_def: ImplDef,
        fn_def: FnDef,
        partial_args: Option<GenericArgs>,
    ) {
        let mut record = self.data.ty_record_mut(ty);
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
        });
    }
}
