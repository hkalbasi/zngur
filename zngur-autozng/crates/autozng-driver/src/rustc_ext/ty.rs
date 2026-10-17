use rustc_public::{
    ty::{RigidTy, Ty, TyConstKind, TyKind},
    visitor::Visitable,
};

pub trait RustcPublicTyExt {
    fn base_ty(&self) -> rustc_public::ty::Ty;
    fn dependent_tys(&self) -> Vec<rustc_public::ty::Ty> {
        let mut tys = Vec::new();
        self.dependent_tys_inner(&mut tys);
        return tys;
    }
    fn dependent_tys_inner(&self, tys: &mut Vec<rustc_public::ty::Ty>);

    /// the type contains no generic params or unevaluated constants
    fn is_concrete(&self) -> bool;
    fn adt_def(&self) -> Option<rustc_public::ty::AdtDef>;
}

impl RustcPublicTyExt for rustc_public::ty::Ty {
    fn base_ty(&self) -> rustc_public::ty::Ty {
        let mut ty = *self;
        while let TyKind::RigidTy(RigidTy::RawPtr(inner_ty, _) | RigidTy::Ref(_, inner_ty, _)) =
            ty.kind()
        {
            ty = inner_ty
        }
        ty
    }

    fn dependent_tys_inner(&self, tys: &mut Vec<rustc_public::ty::Ty>) {
        let kind = self.base_ty().kind();
        match &kind {
            TyKind::RigidTy(ty) => match ty {
                // push inner tys of primitive containers
                RigidTy::Array(ty, _) | RigidTy::Slice(ty) => {
                    tys.push(*ty);
                }
                RigidTy::Tuple(inner_tys) => {
                    tys.extend(inner_tys.iter().copied());
                }
                // push the inputs and outputs of the callable
                RigidTy::FnDef(..) | RigidTy::FnPtr(_) | RigidTy::Closure(..) => {
                    let sig = kind.fn_sig().unwrap().skip_binder();
                    tys.extend(sig.inputs_and_output.iter().copied());
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn is_concrete(&self) -> bool {
        struct GenericTyVisitor;

        // a visitor that breaks on generic or unevaluated params
        impl rustc_public::visitor::Visitor for GenericTyVisitor {
            type Break = ();

            fn visit_ty(&mut self, ty: &Ty) -> std::ops::ControlFlow<Self::Break> {
                if matches!(ty.kind(), TyKind::Param(_)) {
                    std::ops::ControlFlow::Break(())
                } else {
                    ty.super_visit(self)
                }
            }

            fn visit_const(
                &mut self,
                c: &rustc_public::ty::TyConst,
            ) -> std::ops::ControlFlow<Self::Break> {
                if matches!(
                    c.kind(),
                    TyConstKind::Param(_) | TyConstKind::Unevaluated(_, _)
                ) {
                    std::ops::ControlFlow::Break(())
                } else {
                    c.super_visit(self)
                }
            }
        }

        let cf = self.visit(&mut GenericTyVisitor);
        cf.is_continue()
    }

    fn adt_def(&self) -> Option<rustc_public::ty::AdtDef> {
        if let TyKind::RigidTy(RigidTy::Adt(adt_def, _)) = self.base_ty().kind() {
            Some(adt_def)
        } else {
            None
        }
    }
}
