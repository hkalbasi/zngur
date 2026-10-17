//! The accumulation target for the analysis passes.
//!
//! [`AutozngContext`] collects one [`TyRecord`] per visited type (keyed by `Ty`) plus a
//! [`MetadataMap`] of per-item tool attributes keyed by `MetadataKey`. Both passes in
//! `analyze.rs` (the HIR pass and the `rustc_public` pass) write into the same context.
//!
//! Note: [`AutozngContext::record_impl_method`] is defined but currently **never called**,
//! so every [`TyRecord::impls`] is empty and no methods/constructors reach the render step.
//! Populating it is part of wiring up rendering.

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
    /// Keyed by the impl's `DefId`. Empty until `record_impl_method` is called (it isn't, yet).
    pub impls: IndexMap<DefId, ImplRecord>,
}

#[derive(Debug, Default)]
struct AutozngContextData {
    tys: RefCell<FxHashMap<Ty, TyRecord>>,
    metadata: RefCell<MetadataMap>,
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
    pub fn config(&self) -> &Config {
        &self.cfg
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

    /// Record a concrete associated fn as belonging to `ty`.
    ///
    /// **Not currently called from anywhere** (see module docs). `partial_args` is the
    /// partially-applied generic args for methods on generic types; the `clauses` on the
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
