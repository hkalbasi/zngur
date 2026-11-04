use rustc_public::{
    rustc_internal::{internal, stable},
    ty::{GenericArgKind, GenericArgs, ImplDef, RigidTy, TraitRef, Ty, TyConstKind, TyKind},
    visitor::{Visitable, Visitor},
};

use crate::driver::with_ctxt;

pub trait RustcPublicTyExt {
    fn base_ty(&self) -> rustc_public::ty::Ty;

    /// The inherent impls of a **non-nominal** type (a slice, `str`, or a primitive).
    ///
    /// ADTs expose [`rustc_public::ty::AdtDef::inherent_impls`]; rustc keys every other type
    /// through a `SimplifiedType` instead (this is what method resolution uses for primitives).
    fn incoherent_impls(&self) -> Vec<ImplDef>;

    /// The implicit generic args of a slice's inherent impl (`impl<T> [T]`) for this type's
    /// element. `str`'s impl and non-slice types have none.
    fn slice_element_args(&self) -> Option<GenericArgs>;

    #[allow(dead_code)] // collects a type's inner types; not consumed yet
    fn dependent_tys(&self) -> Vec<rustc_public::ty::Ty> {
        let mut tys = Vec::new();
        self.dependent_tys_inner(&mut tys);
        tys
    }
    #[allow(dead_code)] // only reached through `dependent_tys`
    fn dependent_tys_inner(&self, tys: &mut Vec<rustc_public::ty::Ty>);

    /// the type contains no generic params or unevaluated constants
    fn is_concrete(&self) -> bool;
    fn adt_def(&self) -> Option<rustc_public::ty::AdtDef>;

    /// The principal trait of every trait object inside the type.
    ///
    /// A `dyn Trait<ConcreteArgs>` can sit anywhere: behind a reference or raw pointer, inside a
    /// container (`Option<&dyn T>`, `Box<dyn T>`, `PhantomData<dyn T>`), as a function pointer's
    /// input or output, or as a tuple element. The object's *principal* trait is returned with the
    /// object as its `Self` type; auto traits (`dyn T + Send`) and associated-type projections are
    /// not traits a `.zng` `trait` block can represent, so they are skipped.
    ///
    /// Used by `#[autozng::emit_items(emit_traits = true)]` to name concrete trait instantiations
    /// as export targets: a generic trait is only representable in `.zng` once all of its type
    /// parameters are known, so the user enumerates e.g. `let _: &dyn GenericTrait<u32>;`.
    fn trait_objects(&self) -> Vec<TraitRef>;
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

    fn incoherent_impls(&self) -> Vec<ImplDef> {
        use rustc_middle::ty::fast_reject::{TreatParams, simplify_type};

        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let self_ty = internal(tcx, *self);
            let Some(simplified) = simplify_type(tcx, self_ty, TreatParams::InstantiateWithInfer)
            else {
                return Vec::new();
            };
            tcx.incoherent_impls(simplified)
                .iter()
                .map(|&def_id| ImplDef(stable(def_id)))
                .collect()
        })
    }

    fn slice_element_args(&self) -> Option<GenericArgs> {
        match self.base_ty().kind() {
            TyKind::RigidTy(RigidTy::Slice(element)) => {
                Some(GenericArgs(vec![GenericArgKind::Type(element)]))
            }
            _ => None,
        }
    }

    fn dependent_tys_inner(&self, tys: &mut Vec<rustc_public::ty::Ty>) {
        let kind = self.base_ty().kind();
        if let TyKind::RigidTy(ty) = &kind {
            match ty {
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
            }
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

    fn trait_objects(&self) -> Vec<TraitRef> {
        use rustc_public::ty::{Binder, ExistentialPredicate};

        struct TraitObjects(Vec<TraitRef>);

        impl Visitor for TraitObjects {
            type Break = ();

            fn visit_ty(&mut self, ty: &Ty) -> std::ops::ControlFlow<Self::Break> {
                if let TyKind::RigidTy(RigidTy::Dynamic(predicates, _)) = ty.kind()
                    // `rustc` orders the principal trait first; projections and auto traits
                    // follow it and are not exportable `trait` blocks on their own.
                    && let Some(Binder {
                        value: ExistentialPredicate::Trait(trait_ref),
                        ..
                    }) = predicates.into_iter().next()
                {
                    self.0.push(trait_ref.with_self_ty(*ty));
                }
                // Keep descending so a trait object nested in another object's arguments, or in
                // a container/function-pointer type, is found too.
                ty.super_visit(self)
            }
        }

        let mut objects = TraitObjects(Vec::new());
        // The visitor never breaks, so the result is the collected list.
        let _ = self.visit(&mut objects);
        objects.0
    }
}
