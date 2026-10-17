//! Attribute collection and caching, keyed per item `DefId`.
//!
//! [`AttrsFor::collect`] gathers tool attributes for an item (optionally including
//! doc-comment pseudo-attrs) and caches the parsed result by `attr_key`. Then
//! [`AutozngAttrsExt::autozng_attrs`] forks the parent's child-targeted attributes
//! (`rename_all`, `emit(name)`/`skip(name)`) onto the item (via [`Attr::for_child`]) and
//! merges them with the item's own attributes, so a child sees its parent's attributes and
//! can still override them.
//!
//! Note the key chosen for `HirLetBindStmt` below: it is the *enclosing item's* `DefId`,
//! not the `let` statement, so attributes on individual `let` bindings are not collected.

use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

use indexmap::IndexMap;
use rustc_public::{CrateDef, rustc_internal::stable};
use rustc_span::def_id::DefId as RustcDefId;

use crate::{
    attrs::{
        Attr, AttrCollectionExt, AttrPath, AttrTarget, DynAttr,
        macros::impl_has_attrs,
        parse::storage::{HasAttrs, HasParentAttrs, ToolStorage},
        process_attrs,
    },
    driver::with_ctxt,
    error::pull_and_emit_errors_from_results,
    rustc_ext::FieldDefExt,
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
    rustc_public::ty::FnDef { UseCrateDef },
    rustc_public::ty::ImplDef { UseCrateDef },
    rustc_public::ty::TraitDef { UseCrateDef },
    rustc_public::ty::FieldDef {
        UseCrateDef,
        WithParents |&self, _key| {
            // The field's attrs are inherited from its owning ADT (`emit(name)`,
            // `rename_all`; `rename`/`emit` aimed at the ADT itself are dropped by
            // `for_child`).
            self.adt_def().map(|adt| adt.autozng_attrs())
        }
    },
    rustc_public::ty::AssocItem {
       WithAttrKey |&self| { self.def_id.def_id().attr_key() },
       WithParents |&self, key| {
        with_ctxt(|ctx|{
            use rustc_hir::def::DefKind;
            let tcx = ctx.tcx;
            // For an impl item this is the impl block, for a trait item the trait declaration.
            let (parent_id, parent_kind) = tcx.assoc_parent(key)?;
            let mut attrs = Vec::new();
            match parent_kind {
                DefKind::Trait => {
                    attrs.extend(rustc_public::ty::TraitDef(stable(parent_id)).autozng_attrs());
                }
                DefKind::Impl { .. } => {
                    // An impl block's own child-targeted attrs (`emit(name)`, `rename_all`).
                    attrs.extend(rustc_public::ty::ImplDef(stable(parent_id)).autozng_attrs());
                    // ... plus the attrs of the type the impl is for, so e.g. an `emit(name)`
                    // on the type reaches its methods.
                    let self_ty = tcx
                        .type_of(parent_id)
                        .instantiate_identity()
                        .skip_norm_wip();
                    if let Some(adt) = self_ty.ty_adt_def() {
                        attrs.extend(
                            rustc_public::ty::AdtDef(stable(adt.did())).autozng_attrs(),
                        );
                    }
                }
                _ => {}
            }
            (!attrs.is_empty()).then_some(attrs)
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
        let own = AttrsFor::collect(self.attr_key(), self);

        // Attributes a parent aimed at its children (`rename_all`, `emit(name)`/`skip(name)`)
        // are forked onto `self`; a parent's "self" attributes (`emit`/`rename`) are dropped.
        let parent_attrs = self.parent_attrs(self.attr_key()).unwrap_or_default();
        let inherited = (&parent_attrs).for_child(self);
        if inherited.is_empty() {
            return own;
        }

        // Merge inherited attrs with our own by path, keeping first-seen order. The child's
        // own attribute is merged *into* the inherited one, so it wins (e.g. an explicit
        // `skip` overrides an inherited `emit(name)`, and a `rename` overrides `rename_all`).
        let mut merged: IndexMap<AttrPath, Rc<dyn DynAttr>> = IndexMap::new();
        for attr in inherited
            .into_iter()
            .chain(own.into_iter().map(|attr| attr.clone_dyn()))
        {
            match merged.entry(attr.path()) {
                indexmap::map::Entry::Occupied(mut entry) => {
                    // SAFETY: entries are only ever inserted from `for_child`/`clone_dyn`, both
                    // of which yield uniquely-owned `Rc`s that are never cloned afterwards.
                    Rc::get_mut(entry.get_mut())
                        .expect("attribute Rc must be uniquely owned")
                        .merge(attr);
                }
                indexmap::map::Entry::Vacant(entry) => {
                    entry.insert(attr);
                }
            }
        }
        merged.into_values().collect()
    }
}
