use rustc_public::{
    CrateDef,
    rustc_internal::{internal, stable},
    ty::{FnDef, FnSig, GenericArgKind, GenericArgs, ImplDef, TraitDef, TraitRef, Ty},
};

use crate::rustc_ext::{RustcPublicTyExt, convert::stable_trait};
use crate::{driver::with_ctxt, rustc_ext::RustcInternalEx};

/// Extensions for [`FnDef`]s.
pub(crate) trait FnDefExt {
    /// The signature with its early-bound generic parameters replaced by `args`.
    ///
    /// `args` are the generic arguments of the fully monomorphized self type (see
    /// [`crate::render`]); they must cover *all* of the function's generic parameters (the ones
    /// inherited from the impl block plus its own). Callers check that with `generics_of` before
    /// calling, since instantiating with too few arguments would be ill-formed.
    fn instantiate_sig(&self, args: &GenericArgs) -> FnSig;

    /// The impl block (inherent or trait) that this function is an associated item of, together
    /// with the impl's (possibly generic) `Self` type.
    ///
    /// Used when a `let` binding inside a module-level `emit_items` const names an associated
    /// function (e.g. `Example::foo::<u8>`): the function belongs to its own impl rather than
    /// being a free function. Returns `None` for a genuinely free function, or for a method
    /// declared in a trait (whose `Self` is not concrete at that point).
    fn impl_of(&self) -> Option<(Ty, ImplDef)>;

    /// The `Self: Trait` reference of a trait method, if this function is declared in a trait
    /// (rather than an inherent method or a free function).
    ///
    /// `generics` are the fn item's generic args (`FnDef(fn_def, generics)`); for a trait method
    /// they start with `Self`, followed by the trait's own args.
    fn trait_ref(&self, generics: &GenericArgs) -> Option<TraitRef>;
}

impl FnDefExt for FnDef {
    fn instantiate_sig(&self, args: &GenericArgs) -> FnSig {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let def_id = internal(tcx, self.def_id());
            let args = internal(tcx, args);
            stable(
                tcx.fn_sig(def_id)
                    .instantiate(tcx, args)
                    .skip_normalization()
                    .skip_binder(),
            )
        })
    }
    fn impl_of(&self) -> Option<(Ty, ImplDef)> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let impl_def_id = tcx.impl_of_assoc(internal(tcx, self.def_id()))?;
            let impl_def = ImplDef(stable(impl_def_id));
            let self_ty = stable(
                tcx.type_of(impl_def_id)
                    .instantiate_identity()
                    .skip_norm_wip(),
            );
            Some((self_ty.base_ty(), impl_def))
        })
    }

    fn trait_ref(&self, generics: &GenericArgs) -> Option<TraitRef> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let (parent_id, parent_kind) = tcx.assoc_parent(internal(tcx, self.def_id()))?;
            if !matches!(parent_kind, rustc_hir::def::DefKind::Trait) {
                return None;
            }
            let trait_def = TraitDef(stable(parent_id));
            let mut args = generics.0.iter();
            let self_ty = match args.next()? {
                GenericArgKind::Type(ty) => *ty,
                _ => return None,
            };
            let rest = GenericArgs(args.cloned().collect());
            Some(TraitRef::new(trait_def, self_ty, &rest))
        })
    }
}

pub(crate) trait GetAssocInfoExt {
    #[allow(dead_code)] // kept for inspecting a trait/impl method's signature
    fn opt_fn_sig(&self) -> Option<rustc_public::ty::FnSig>;
    fn opt_fn_def(&self) -> Option<(rustc_public::ty::FnDef, GenericArgs)>;
    fn opt_trait_def(&self) -> Option<rustc_public::ty::TraitDef>;
    fn opt_trait_ref(&self) -> Option<rustc_public::ty::TraitRef>;
}

impl GetAssocInfoExt for rustc_public::ty::AssocItem {
    fn opt_fn_sig(&self) -> Option<rustc_public::ty::FnSig> {
        with_ctxt(|ctx| match &self.kind {
            rustc_public::ty::AssocKind::Fn { .. } => {
                let rustc_def_id = internal(ctx.tcx, self.def_id.def_id());
                Some(stable(
                    ctx.tcx
                        .fn_sig(rustc_def_id)
                        .instantiate_identity()
                        .skip_normalization()
                        .skip_binder(),
                ))
            }
            _ => None,
        })
    }
    fn opt_fn_def(&self) -> Option<(rustc_public::ty::FnDef, GenericArgs)> {
        with_ctxt(|ctx| {
            stable(ctx.tcx.type_of(self.def_id.def_id().internal(ctx.tcx)))
                .value
                .kind()
                .fn_def()
                .map(|(fn_def, generics)| (fn_def, generics.clone()))
        })
    }

    fn opt_trait_def(&self) -> Option<rustc_public::ty::TraitDef> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let def_id = self.def_id.def_id().internal(tcx);
            let (parent, kind) = tcx.assoc_parent(def_id)?;
            match kind {
                rustc_hir::def::DefKind::Trait => Some(stable_trait(tcx.trait_def(parent)).def_id),
                rustc_hir::def::DefKind::Impl { of_trait } => {
                    if of_trait {
                        let tref = stable(
                            tcx.impl_trait_ref(parent)
                                .instantiate_identity()
                                .skip_normalization(),
                        );
                        Some(tref.def_id)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        })
    }
    fn opt_trait_ref(&self) -> Option<rustc_public::ty::TraitRef> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let def_id = self.def_id.def_id().internal(tcx);
            let (parent, kind) = tcx.assoc_parent(def_id)?;
            match kind {
                rustc_hir::def::DefKind::Impl { of_trait } => {
                    if of_trait {
                        let tref = stable(
                            tcx.impl_trait_ref(parent)
                                .instantiate_identity()
                                .skip_normalization(),
                        );
                        Some(tref)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        })
    }
}
