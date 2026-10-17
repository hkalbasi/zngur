//! Attribute collection and caching, keyed per item `DefId`.
//!
//! [`AttrsFor::collect`] gathers tool attributes for an item (optionally including
//! doc-comment pseudo-attrs) and caches the parsed result by `attr_key`, then
//! [`AutozngAttrsExt::autozng_attrs`] chains in parent attributes.
//!
//! Note the key chosen for `HirLetBindStmt` below: it is the *enclosing item's* `DefId`,
//! not the `let` statement, so attributes on individual `let` bindings are not collected.

use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

use rustc_public::{CrateDef, rustc_internal::stable};
use rustc_span::def_id::DefId as RustcDefId;

use crate::{
    attrs::{
        Attr, AttrTarget, DynAttr,
        macros::impl_has_attrs,
        parse::storage::{HasAttrs, HasParentAttrs, ToolStorage},
        process_attrs,
    },
    driver::with_ctxt,
    error::pull_and_emit_errors_from_results,
};

thread_local! {
    static ATTRCACHE: RefCell<HashMap<RustcDefId, Box<[Rc<dyn DynAttr>]>>> = RefCell::new(HashMap::new());
}

pub struct AttrsFor<ID: HasAttrs>(PhantomData<ID>);

impl<ID: HasAttrs> AttrsFor<ID> {
    pub fn collect(id: ID, item: &dyn AttrTarget) -> Vec<Rc<dyn DynAttr>> {
        ATTRCACHE.with_borrow_mut(|cache| {
            cache
                .entry(id.attr_key())
                .or_insert_with(|| {
                    let prepared = with_ctxt(|ctx| {
                        if ctx.cfg.use_doc_comment_tool_attrs {
                            crate::attrs::parse::storage::WithDocCommentAttrs::build()
                                .collect_attrs_for(id)
                                .prepare_attrs()
                        } else {
                            crate::attrs::parse::storage::ToolAttrsOnly::build()
                                .collect_attrs_for(id)
                                .prepare_attrs()
                        }
                    });
                    process_attrs(prepared, item).into()
                })
                .to_vec()
        })
    }
}

impl_has_attrs! {
    rustc_public::Crate { WithAttrKey |&self| { self.id.attr_key() } },
    rustc_public::ty::AdtDef { UseCrateDef },
    rustc_public::ty::TraitDef { UseCrateDef },
    rustc_public::ty::AssocItem {
       WithAttrKey |&self| { self.def_id.def_id().attr_key() },
       WithParents |&self, key| {
        with_ctxt(|ctx|{
            let tcx = ctx.tcx;
            tcx
                .impl_of_assoc(key)
                .and_then(|parent_id| {
                    use rustc_hir::def::DefKind;
                    match tcx.def_kind(parent_id) {
                        DefKind::Struct | DefKind::Union | DefKind::Enum => {
                            Some(rustc_public::ty::AdtDef(stable(parent_id)).autozng_attrs())
                        }
                        DefKind::Trait => {
                            Some(rustc_public::ty::TraitDef(stable(parent_id)).autozng_attrs())
                        }
                        _ => None,
                    }
                })
        })
    }},
    crate::rustc_ext::HirConstItem { WithAttrKey |&self| { self.owner_id.to_def_id() } },
    // Pitfall: resolves to the enclosing item, not the `let` binding (see module docs).
    crate::rustc_ext::HirLetBindStmt { WithAttrKey |&self| { self.hir_id.owner.to_def_id() } },
    crate::rustc_ext::HirTyAlias { WithAttrKey |&self| { self.owner_id.to_def_id() } }
}

pub(crate) trait AutozngAttrsExt {
    fn autozng_attrs(&self) -> Vec<Rc<dyn DynAttr>>;
}

impl<T: HasAttrs + HasParentAttrs + AttrTarget> AutozngAttrsExt for T {
    fn autozng_attrs(&self) -> Vec<Rc<dyn DynAttr>> {
        self.parent_attrs(self.attr_key())
            .into_iter()
            .flatten()
            .chain(AttrsFor::collect(self.attr_key(), self))
            .collect()
    }
}
