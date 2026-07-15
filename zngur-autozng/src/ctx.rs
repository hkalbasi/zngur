use rustc_middle::ty::TyCtxt;

use rustc_middle::ty::Visibility;
use rustc_public::DefId;
use rustc_span::{Span, def_id::DefId as RustcDefId};

use std::collections::HashMap;
use std::sync::Mutex;

enum Cached<T> {
    Missing,
    Fetched(T),
}

pub struct AutoZngTyMetadata {
    docs: Cached<Vec<(String, Span)>>,
}

impl AutoZngTyMetadata {
    pub fn new() -> Self {
        AutoZngTyMetadata {
            docs: Cached::Missing,
        }
    }
}

impl Default for AutoZngTyMetadata {
    fn default() -> Self {
        AutoZngTyMetadata::new()
    }
}

pub struct AutoZngContext<'tcx> {
    tcx: TyCtxt<'tcx>,
    vis_cache: Mutex<HashMap<DefId, Visibility>>,
    meta_cache: Mutex<HashMap<DefId, AutoZngTyMetadata>>,
    owner_docs_cache: Mutex<HashMap<RustcDefId, Vec<(String, Span)>>>,
}

impl<'tcx> AutoZngContext<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        AutoZngContext {
            tcx,
            vis_cache: Mutex::new(HashMap::new()),
            meta_cache: Mutex::new(HashMap::new()),
            owner_docs_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Check if the provided def is public to the local crate's external API
    /// *WILL* panic if the def doesn't have a visibility (like a generic type param)
    /// the `tcx` is not UnwindSafe so there is no way to make this function panic safe
    /// call with care.
    pub fn is_public_vis(&self, def_id: DefId) -> bool {
        let mut vis_cache = self.vis_cache.lock().expect("poisoned context!");
        let vis = vis_cache.entry(def_id).or_insert_with(|| {
            if let Some(rustc_def_id) =
                rustc_public::rustc_internal::internal(self.tcx, def_id).as_local()
            {
                // SAFETY: asserted local above
                let vis = self.tcx.visibility(rustc_def_id).expect_local();
                vis
            } else {
                Visibility::Public // if the def isn't local it's effectively is public by definition, compilation would fail otherwise
            }
        });
        vis.is_public()
    }

    /// collect docs for a def into a vec of (line, span) pairs
    pub fn docs_for(&self, def_id: DefId) -> Vec<(String, Span)> {
        let mut meta_cache = self.meta_cache.lock().expect("poisoned contex!");
        let metadata = meta_cache
            .entry(def_id)
            .or_insert_with(|| AutoZngTyMetadata::default());
        let docs = match &metadata.docs {
            Cached::Missing => {
                let rustc_def_id = rustc_public::rustc_internal::internal(self.tcx, def_id);
                let docs = docs_for(&self.tcx, rustc_def_id);
                metadata.docs = Cached::Fetched(docs.clone());
                docs
            }
            Cached::Fetched(docs) => docs.clone(),
        };
        docs
    }

    pub fn docs_for_owner(&'_ self, def_id: DefId) -> Option<Vec<(String, Span)>> {
        let rustc_def_id = rustc_public::rustc_internal::internal(self.tcx, def_id);
        let docs = self.tcx.impl_of_assoc(rustc_def_id).map(|impl_def_id| {
            let mut owner_docs_cache = self.owner_docs_cache.lock().expect("poisoned contex!");
            let docs = owner_docs_cache
                .entry(impl_def_id)
                .or_insert_with(|| docs_for(&self.tcx, impl_def_id));
            docs.clone()
        });
        docs
    }
}

fn docs_for<'tcx, ID: rustc_hir::attrs::HasAttrs<'tcx, TyCtxt<'tcx>>>(
    tcx: &TyCtxt<'tcx>,
    id: ID,
) -> Vec<(String, Span)> {
    let attrs = rustc_hir::attrs::HasAttrs::get_attrs(id, tcx);
    let mut docs = Vec::new();
    for attr in attrs {
        use rustc_hir::attrs::AttributeKind::*;
        let attr: &rustc_hir::Attribute = attr;
        match attr {
            rustc_hir::Attribute::Parsed(DocComment { comment, span, .. }) => {
                docs.push((comment.to_string(), span.clone()));
            }
            rustc_hir::Attribute::Unparsed(_attr_item) => {} // not a doc comment

            #[deny(unreachable_patterns)]
            _ => {}
        }
    }
    docs
}
