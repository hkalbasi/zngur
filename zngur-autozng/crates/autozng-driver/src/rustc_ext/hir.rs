use rustc_public::{CrateDef, rustc_internal::stable};

use crate::{
    driver::with_ctxt,
    metadata::{IntoMetadataKey, MetadataKey},
    rustc_ext::{
        GenericClauses, RustcInternalEx,
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

#[derive(Debug, Clone)]
pub struct HirLetBindStmt {
    pub parent: rustc_hir::OwnerId,
    pub hir_id: rustc_hir::HirId,
    pub ty: rustc_public::ty::Ty,
}

impl IntoMetadataKey for HirLetBindStmt {
    fn to_metadata_key(&self) -> MetadataKey {
        self.hir_id.owner.to_metadata_key()
    }
}

impl HirConstItem {
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

    pub fn inner_emit_types(&self) -> Vec<rustc_public::ty::Ty> {
        use rustc_public::ty::{RigidTy, TyKind};
        match self.ty.kind() {
            TyKind::RigidTy(RigidTy::Array(ty, _) | RigidTy::Slice(ty)) => Some(ty),
            _ => None,
        }
        .and_then(|ty| {
            if let TyKind::RigidTy(RigidTy::Tuple(tys)) = ty.kind() {
                Some(tys)
            } else {
                None
            }
        })
        .unwrap_or_default()
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
