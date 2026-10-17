use rustc_public::{CrateDef, rustc_internal::stable};

use crate::{
    driver::with_ctxt,
    metadata::{IntoMetadataKey, MetadataKey},
    rustc_ext::{
        GenericClauses, RustcInternalEx, RustcPublicTyExt,
        generics::{get_clauses_of, get_generics_of},
    },
};

#[derive(Debug, Clone)]
pub struct HirConstItem {
    pub owner_id: rustc_hir::OwnerId,
    pub ty: rustc_public::ty::Ty,
    pub body_id: Option<rustc_hir::BodyId>,
}

impl IntoMetadataKey for HirConstItem {
    fn to_metadata_key(&self) -> MetadataKey {
        self.owner_id.to_metadata_key()
    }
}

/// Per-`let` binding discovered inside a `const` block (used for `emit_items`).
///
/// Its attribute key is the statement's own `HirId` (rustc lowers statement attributes with
/// `Target::Statement`), so a `#[autozng::...]` on the binding is collected; see
/// `attrs/attrs_for.rs`. Its *metadata* key resolves to the function item the binding names,
/// so a `#[autozng::rename]` reaches the emitted method (see [`Self::to_metadata_key`]).
#[derive(Debug, Clone)]
pub struct HirLetBindStmt {
    pub parent: rustc_hir::OwnerId,
    pub hir_id: rustc_hir::HirId,
    pub ty: rustc_public::ty::Ty,
}

impl IntoMetadataKey for HirLetBindStmt {
    fn to_metadata_key(&self) -> MetadataKey {
        use rustc_public::ty::{RigidTy, TyKind};

        // A `let` binding in an `emit_items` const names a function item (a monomorphization, an
        // associated fn, or a trait method). Its `rename` must reach *that* function's emitted
        // record, so the export name is keyed the way `render` looks it up: by the fn's `DefId`
        // plus the enumerated generic args (the same list stored as `FnRecord::partial_args` /
        // `TraitFnRecord::generics`). An empty arg list matches a record with `partial_args =
        // None`, which `render` looks up by the plain fn `DefId`.
        if let TyKind::RigidTy(RigidTy::FnDef(fn_def, generics)) = self.ty.kind() {
            if generics.0.is_empty() {
                return fn_def.to_metadata_key();
            }
            return MetadataKey::for_monomorphization(fn_def.def_id(), &generics);
        }

        // Not a function item: nothing renames it; keep it under the enclosing item's key.
        self.hir_id.owner.to_metadata_key()
    }
}

impl HirLetBindStmt {
    /// The inherent impl block that owns the `const` this `let` binding lives in, if any.
    ///
    /// A `let` binding inside an `#[autozng::emit_items]` const that is an associated const of
    /// an impl names a monomorphization of one of that impl's methods (e.g.
    /// `let _ = Self::foo::<u32>;`). The enclosing impl supplies the (possibly generic) `Self`
    /// type and the impl's clauses. Returns `None` for a const at module scope.
    pub fn impl_parent(&self) -> Option<(rustc_public::ty::Ty, rustc_public::ty::ImplDef)> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let parent_def_id = tcx.parent(self.parent.def_id.to_def_id());
            if !matches!(
                tcx.def_kind(parent_def_id),
                rustc_hir::def::DefKind::Impl { .. }
            ) {
                return None;
            }
            let impl_def = rustc_public::ty::ImplDef(stable(parent_def_id));
            let self_ty =
                stable(tcx.type_of(parent_def_id).instantiate_identity().skip_norm_wip());
            Some((self_ty.base_ty(), impl_def))
        })
    }
}

impl HirConstItem {
    /// The span of the `const` item itself (the item tagged `#[autozng::emit_items]`).
    ///
    /// Used as the recorded span of the types the `const` enumerates: unlike an ADT or a
    /// function, an enumerated slice/tuple/primitive has no source span of its own.
    pub fn span(&self) -> rustc_span::Span {
        with_ctxt(|ctx| ctx.tcx.def_span(self.owner_id.def_id))
    }

    pub fn body_let_bind_stmts(&self) -> Vec<HirLetBindStmt> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            if let Some(rustc_hir::Expr {
                kind:
                    rustc_hir::ExprKind::Block(
                        rustc_hir::Block {
                            stmts, expr: None, ..
                        },
                        _,
                    ),
                ..
            }) = self.body_id.map(|body_id| *tcx.hir_body(body_id).value)
            {
                Some(stmts.iter().filter_map(|stmt| {
                    if let rustc_hir::StmtKind::Let(rustc_hir::LetStmt{
                        pat: rustc_hir::Pat {
                            kind: rustc_hir::PatKind::Wild | rustc_hir::PatKind::Binding(..),
                            ..
                        },
                        init: stmt_init,
                        ty: stmt_ty,
                        ..
                    }) = stmt.kind {
                        if stmt_ty.is_some() {
                            stmt_ty.map(|ty| stable(rustc_hir_analysis::lower_ty(tcx, ty)))
                        } else if let Some(rustc_hir::Expr {
                            hir_id: stmt_expr_id, ..
                        }) = stmt_init {
                            tcx.typeck(stmt.hir_id.owner.def_id).node_type_opt(*stmt_expr_id).map(stable)
                        } else {
                            None
                        }.map(|ty|{
                            HirLetBindStmt {
                                parent: self.owner_id,
                                hir_id: stmt.hir_id,
                                ty
                            }
                        })
                    } else {
                        None
                    }
                }).collect())
            } else {
                None
            }
        }).unwrap_or_default()
    }

    /// The types directly enumerated inside a `const` tagged `emit_items`.
    ///
    /// The const's type is a list of the types to emit, written as a tuple and optionally
    /// wrapped in a reference / array / slice: `(A, B)`, `[(A, B); 0]` and `&[(A, B)]` all
    /// enumerate `A` and `B`. References (and raw pointers) are peeled with `base_ty`, then
    /// the array/slice element (or the tuple itself) supplies the elements.
    ///
    /// Other shapes (a bare non-tuple type, `Option<(A, B)>`, ...) are not recognized; the
    /// `let`-binding form is handled separately by [`Self::body_let_bind_stmts`].
    pub fn inner_emit_types(&self) -> Vec<rustc_public::ty::Ty> {
        use rustc_public::ty::{RigidTy, TyKind};

        // Peel `&`/`*const`/`*mut` wrappers, e.g. `&[(A, B)]` -> `[(A, B)]`.
        let ty = self.ty.base_ty();
        let tuple = match ty.kind() {
            TyKind::RigidTy(RigidTy::Array(element, _) | RigidTy::Slice(element)) => element,
            TyKind::RigidTy(RigidTy::Tuple(_)) => ty,
            _ => return Vec::new(),
        };
        match tuple.kind() {
            TyKind::RigidTy(RigidTy::Tuple(tys)) => tys,
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HirTyAlias {
    pub owner_id: rustc_hir::OwnerId,
    pub ident: rustc_span::Ident,
    pub generics: rustc_public::ty::Generics,
    pub clauses: GenericClauses,
    pub ty: rustc_public::ty::Ty,
}

impl HirTyAlias {}

impl IntoMetadataKey for HirTyAlias {
    fn to_metadata_key(&self) -> MetadataKey {
        self.owner_id.to_metadata_key()
    }
}

pub trait HirConstBodyExt {
    fn opt_const_item(&self) -> Option<HirConstItem>;
}
pub trait HirItemExt {
    fn opt_ty_alias(&self) -> Option<HirTyAlias>;
}

pub trait HirTyExt {
    fn ty(&self) -> Option<rustc_public::ty::Ty>;
}

impl<'hir> HirTyExt for rustc_hir::Item<'hir> {
    fn ty(&self) -> Option<rustc_public::ty::Ty> {
        with_ctxt(|ctx| {
            let typeck_results = ctx.tcx.typeck(self.owner_id.def_id);
            let ty = typeck_results.node_type_opt(self.hir_id());
            ty.map(stable)
        })
    }
}

impl<'hir> HirItemExt for rustc_hir::Item<'hir> {
    fn opt_ty_alias(&self) -> Option<HirTyAlias> {
        if let rustc_hir::Item {
            owner_id,
            kind: rustc_hir::ItemKind::TyAlias(ident, _, ty),
            ..
        } = self
        {
            let def_id = stable(owner_id.to_def_id());
            let generics = get_generics_of(def_id);
            let clauses = get_clauses_of(def_id);

            Some(HirTyAlias {
                owner_id: *owner_id,
                ident: *ident,
                generics,
                clauses,
                ty: with_ctxt(|ctx| stable(rustc_hir_analysis::lower_ty(ctx.tcx, ty))),
            })
        } else {
            None
        }
    }
}

impl<'hir> HirConstBodyExt for rustc_hir::Item<'hir> {
    fn opt_const_item(&self) -> Option<HirConstItem> {
        if let rustc_hir::Item {
            owner_id,
            kind: rustc_hir::ItemKind::Const(_, _, ty, rhs),
            ..
        } = self
        {
            Some(HirConstItem {
                owner_id: *owner_id,
                ty: { with_ctxt(|ctx| stable(rustc_hir_analysis::lower_ty(ctx.tcx, ty))) },
                body_id: match rhs {
                    rustc_hir::ConstItemRhs::Body(body_id) => Some(*body_id),
                    rustc_hir::ConstItemRhs::TypeConst(_) => None,
                },
            })
        } else {
            None
        }
    }
}

impl<'hir> HirTyExt for rustc_hir::ImplItem<'hir> {
    fn ty(&self) -> Option<rustc_public::ty::Ty> {
        with_ctxt(|ctx| {
            let typeck_results = ctx.tcx.typeck(self.owner_id.def_id);
            let ty = typeck_results.node_type_opt(self.hir_id());
            ty.map(stable)
        })
    }
}

impl<'hir> HirConstBodyExt for rustc_hir::ImplItem<'hir> {
    fn opt_const_item(&self) -> Option<HirConstItem> {
        if let rustc_hir::ImplItem {
            owner_id,
            impl_kind: rustc_hir::ImplItemImplKind::Inherent { .. },
            kind: rustc_hir::ImplItemKind::Const(ty, rhs),
            ..
        } = self
        {
            Some(HirConstItem {
                owner_id: *owner_id,
                ty: { with_ctxt(|ctx| stable(rustc_hir_analysis::lower_ty(ctx.tcx, ty))) },
                body_id: match rhs {
                    rustc_hir::ConstItemRhs::Body(body_id) => Some(*body_id),
                    rustc_hir::ConstItemRhs::TypeConst(_) => None,
                },
            })
        } else {
            None
        }
    }
}

// impl HirConstBodyExt for rustc_public::CrateItem {
//     fn opt_const_item(&self) -> Option<HirConstItem> {
//         if let rustc_public::ItemKind::Const = self.kind() {
//             with_tcx(|tcx| {
//                 self.def_id()
//                     .internal(tcx)
//                     .as_local()
//                     .and_then(|local_def_id| {
//                         if let rustc_hir::Node::Item(rustc_hir::Item {
//                             owner_id,
//                             kind: rustc_hir::ItemKind::Const(_, _, ty, rhs),
//                             ..
//                         }) = tcx.hir_node_by_def_id(local_def_id)
//                         {
//                             Some(HirConstItem {
//                                 owner_id: *owner_id,
//                                 ty: { stable(rustc_hir_analysis::lower_ty(tcx, ty)) },
//                                 body_id: match rhs {
//                                     rustc_hir::ConstItemRhs::Body(body_id) => Some(*body_id),
//                                     rustc_hir::ConstItemRhs::TypeConst(_) => None,
//                                 },
//                             })
//                         } else {
//                             None
//                         }
//                     })
//             })
//         } else {
//             None
//         }
//     }
// }
