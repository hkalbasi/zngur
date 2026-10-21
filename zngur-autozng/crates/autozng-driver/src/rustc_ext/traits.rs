use rustc_middle::ty::ParamEnv;
use rustc_public::{
    CrateDef,
    rustc_internal::stable,
    ty::{GenericArgs, ImplDef, TraitDef, TraitRef, Ty},
};
use rustc_span::DUMMY_SP;

use crate::{
    driver::with_ctxt,
    rustc_ext::{GenericClauses, RustcInternalEx},
};

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum WellKnownTrait {
    Copy,
    Unsized,
    Debug,
    Drop,
    PartialEq,
    PartialOrd,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    IntoIterator,
}

const WELLKNOWNTRAITS: &[WellKnownTrait] = {
    use WellKnownTrait::*;
    &[
        Copy,
        Unsized,
        Debug,
        Drop,
        PartialEq,
        PartialOrd,
        Add,
        Sub,
        Mul,
        Div,
        Rem,
        BitAnd,
        BitOr,
        BitXor,
        Shl,
        Shr,
        IntoIterator,
    ]
};

impl WellKnownTrait {
    /// The `TraitDecl` for this well-known trait, resolved once per analysis and cached.
    ///
    /// `Debug` and `IntoIterator` have no lang item, so they are found by scanning every trait in
    /// the crate graph for a def path; that is far too expensive to repeat for every type (and
    /// every trait impl), so the result is cached on the [`DriverContext`]. See
    /// [`WellKnownTrait::resolve_decl`] for the resolution itself.
    ///
    /// [`DriverContext`]: crate::driver::DriverContext
    pub fn trait_decl(&self) -> rustc_public::ty::TraitDecl {
        with_ctxt(|ctx| ctx.wellknown_trait_decl(*self))
    }

    /// Resolve this well-known trait's declaration against `tcx` (uncached).
    ///
    /// Prefer [`WellKnownTrait::trait_decl`], which caches the result.
    pub(crate) fn resolve_decl(
        &self,
        tcx: rustc_middle::ty::TyCtxt<'_>,
    ) -> rustc_public::ty::TraitDecl {
        let def_id = match self {
            Self::Debug => tcx
                .all_traits_including_private()
                .find(|def_id| tcx.def_path_str(*def_id) == "std::fmt::Debug")
                .expect("std::fmt::Debug is not defined"),
            Self::Copy => tcx.require_lang_item(rustc_attr_ir::LangItem::Copy, DUMMY_SP),
            Self::Unsized => tcx.require_lang_item(rustc_attr_ir::LangItem::Sized, DUMMY_SP),
            Self::PartialEq => tcx.require_lang_item(rustc_attr_ir::LangItem::PartialEq, DUMMY_SP),
            Self::PartialOrd => {
                tcx.require_lang_item(rustc_attr_ir::LangItem::PartialOrd, DUMMY_SP)
            }
            Self::Add => tcx.require_lang_item(rustc_attr_ir::LangItem::Add, DUMMY_SP),
            Self::Sub => tcx.require_lang_item(rustc_attr_ir::LangItem::Sub, DUMMY_SP),
            Self::Mul => tcx.require_lang_item(rustc_attr_ir::LangItem::Mul, DUMMY_SP),
            Self::Div => tcx.require_lang_item(rustc_attr_ir::LangItem::Div, DUMMY_SP),
            Self::Rem => tcx.require_lang_item(rustc_attr_ir::LangItem::Rem, DUMMY_SP),
            Self::BitAnd => tcx.require_lang_item(rustc_attr_ir::LangItem::BitAnd, DUMMY_SP),
            Self::BitOr => tcx.require_lang_item(rustc_attr_ir::LangItem::BitOr, DUMMY_SP),
            Self::BitXor => tcx.require_lang_item(rustc_attr_ir::LangItem::BitXor, DUMMY_SP),
            Self::Shl => tcx.require_lang_item(rustc_attr_ir::LangItem::Shl, DUMMY_SP),
            Self::Shr => tcx.require_lang_item(rustc_attr_ir::LangItem::Shr, DUMMY_SP),
            Self::Drop => tcx.require_lang_item(rustc_attr_ir::LangItem::Drop, DUMMY_SP),
            Self::IntoIterator => tcx
                .all_traits_including_private()
                .find(|def_id| tcx.def_path_str(*def_id) == "std::iter::IntoIterator")
                .expect("std::iter::IntoIterator is not defined"),
        };
        stable(tcx.trait_def(def_id))
    }

    pub fn must_impl_trait(&self) -> bool {
        !matches!(self, Self::Unsized)
    }

    pub fn need_generic(&self) -> bool {
        matches!(
            self,
            Self::PartialEq
                | Self::PartialOrd
                | Self::Add
                | Self::Sub
                | Self::Mul
                | Self::Div
                | Self::Rem
                | Self::BitAnd
                | Self::BitOr
                | Self::BitXor
                | Self::Shl
                | Self::Shr
                | Self::Drop
                | Self::IntoIterator
        )
    }
}

pub trait WellKnownTraitsExt {
    fn implemented_wellknown_traits(&self) -> Vec<(WellKnownTrait, TraitRef)>;
}

impl WellKnownTraitsExt for Ty {
    fn implemented_wellknown_traits(&self) -> Vec<(WellKnownTrait, TraitRef)> {
        with_ctxt(|ctx| {
            use rustc_middle::ty::TypingMode;
            use rustc_trait_selection::infer::InferCtxtExt;
            use rustc_trait_selection::infer::TyCtxtInferExt;

            let tcx = ctx.tcx;

            let infcx = tcx
                .infer_ctxt()
                .with_next_trait_solver(true)
                .ignoring_regions()
                .build(TypingMode::Codegen);

            let rustc_ty = self.internal(tcx);
            let param_env = ParamEnv::empty();

            let mut results: Vec<(WellKnownTrait, TraitRef)> = WELLKNOWNTRAITS
                .iter()
                .copied()
                .filter_map(|trait_enm| {
                    if trait_enm.need_generic() {
                        return None;
                    }
                    let trait_decl = trait_enm.trait_decl();
                    let trait_def_id = trait_decl.def_id.def_id().internal(tcx);
                    let result = infcx
                        .type_implements_trait(trait_def_id, [rustc_ty], param_env)
                        .must_apply_modulo_regions();
                    if (trait_enm.must_impl_trait() && result)
                        || (!trait_enm.must_impl_trait() && !result)
                    {
                        Some((
                            trait_enm,
                            TraitRef::new(trait_decl.def_id, *self, &GenericArgs(Vec::new())),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            // Traits with their own generic parameters (`PartialEq<Rhs>`, `Add<Rhs>`,
            // `IntoIterator`, ...): `Self` alone does not determine the implementation, so the
            // trait is reported when an impl of it has exactly this `Self` type. Only that
            // trait's impls are searched (`tcx.all_impls`, a cached query) rather than every impl
            // in the crate graph, and the `Self` types are compared internally so the
            // `rustc_public` conversion only runs for the impls that actually match.
            for trait_enm in WELLKNOWNTRAITS
                .iter()
                .copied()
                .filter(|trt| trt.need_generic())
            {
                let trait_decl = trait_enm.trait_decl();
                let trait_def_id = trait_decl.def_id.def_id().internal(tcx);
                for impl_id in tcx.all_impls(trait_def_id) {
                    if !is_trait_impl(tcx, impl_id) {
                        continue;
                    }
                    let impl_self_ty = tcx
                        .type_of(impl_id)
                        .instantiate_identity()
                        .skip_normalization();
                    if impl_self_ty != rustc_ty {
                        continue;
                    }
                    let trt_ref = stable(
                        tcx.impl_trait_ref(impl_id)
                            .instantiate_identity()
                            .skip_normalization(),
                    );
                    results.push((trait_enm, trt_ref));
                }
            }
            results
        })
    }
}

/// Extensions for [`ImplDef`]s.
pub(crate) trait ImplDefExt {
    /// The `Self: Trait<..>` reference when this is a trait impl (`None` for an inherent impl).
    fn opt_trait_ref(&self) -> Option<TraitRef>;
}

impl ImplDefExt for ImplDef {
    fn opt_trait_ref(&self) -> Option<TraitRef> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let def_id = self.def_id().internal(tcx);
            if !is_trait_impl(tcx, def_id) {
                return None;
            }
            Some(stable(
                tcx.impl_trait_ref(def_id)
                    .instantiate_identity()
                    .skip_normalization(),
            ))
        })
    }
}

/// Resolving a `#[autozng::emit_on_implementers]` record against a type.
pub(crate) trait EmitForExt {
    /// The concrete `Self: Trait<..>` reference for an `emit_on_implementers` record, when `self`
    /// implements the trait and the record's clauses hold.
    ///
    /// `impl_def` is the record's impl block (`None` for a trait declaration); `clauses` are the
    /// trait's own clauses (declaration case) or the impl block's clauses (impl case). Returns
    /// `None` when the record does not apply to `self`.
    fn resolve_emit_for(
        &self,
        trait_def: TraitDef,
        impl_def: Option<ImplDef>,
        clauses: &GenericClauses,
    ) -> Option<TraitRef>;
}

impl EmitForExt for Ty {
    fn resolve_emit_for(
        &self,
        trait_def: TraitDef,
        impl_def: Option<ImplDef>,
        clauses: &GenericClauses,
    ) -> Option<TraitRef> {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let self_ty = self.internal(tcx);

            // The concrete `Self: Trait<..>` reference and the generic arguments the record's
            // clauses must be instantiated with.
            let (trait_ref, clause_args) = match impl_def {
                Some(impl_def) => {
                    let impl_def_id = impl_def.def_id().internal(tcx);
                    let impl_args = impl_args_matching(tcx, impl_def_id, self_ty)?;
                    let trait_ref = tcx
                        .impl_trait_ref(impl_def_id)
                        .instantiate(tcx, impl_args)
                        .skip_normalization();
                    (stable(trait_ref), stable_generic_args(impl_args))
                }
                None => {
                    let trait_ref = trait_ref_for_decl(tcx, trait_def, self_ty)?;
                    let args = trait_ref.args().clone();
                    (trait_ref, args)
                }
            };

            if !type_implements(tcx, &trait_ref.clone().internal(tcx)) {
                return None;
            }
            if !clauses.clone().instantiate(&clause_args).all_hold() {
                return None;
            }
            Some(trait_ref)
        })
    }
}

/// Whether `impl_def_id` is a trait impl (rather than an inherent one).
fn is_trait_impl(
    tcx: rustc_middle::ty::TyCtxt<'_>,
    impl_def_id: rustc_span::def_id::DefId,
) -> bool {
    matches!(
        tcx.def_kind(impl_def_id),
        rustc_hir::def::DefKind::Impl { of_trait: true }
    )
}

/// Whether the trait has no own generic parameters beyond `Self` (so `Self: Trait` needs no impl
/// lookup to be fully determined).
fn trait_has_no_own_params(
    tcx: rustc_middle::ty::TyCtxt<'_>,
    trait_def_id: rustc_span::def_id::DefId,
) -> bool {
    tcx.generics_of(trait_def_id)
        .own_params
        .iter()
        .all(|param| param.index == 0)
}

/// The concrete `Self: Trait<..>` reference for a trait declaration, resolved against `self_ty`.
///
/// A non-generic trait is fully determined by the trait and the self type. A generic trait is
/// resolved through an impl of it whose `Self` type matches `self_ty` (the impl supplies the
/// concrete trait arguments).
fn trait_ref_for_decl<'tcx>(
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    trait_def: TraitDef,
    self_ty: rustc_middle::ty::Ty<'tcx>,
) -> Option<TraitRef> {
    let trait_def_id = trait_def.def_id().internal(tcx);
    if trait_has_no_own_params(tcx, trait_def_id) {
        return Some(TraitRef::new(
            trait_def,
            stable(self_ty),
            &GenericArgs(Vec::new()),
        ));
    }

    let (impl_def_id, impl_args) = tcx
        .all_impls(trait_def_id)
        .filter(|impl_def_id| is_trait_impl(tcx, *impl_def_id))
        .find_map(|impl_def_id| {
            Some((impl_def_id, impl_args_matching(tcx, impl_def_id, self_ty)?))
        })?;
    Some(stable(
        tcx.impl_trait_ref(impl_def_id)
            .instantiate(tcx, impl_args)
            .skip_normalization(),
    ))
}

/// Convert internal generic arguments to their public [`GenericArgs`] representation.
///
/// `rustc_internal::stable` is only implemented for the list *header*, not the `GenericArgsRef`
/// reference, so the arguments are converted one by one.
fn stable_generic_args(args: rustc_middle::ty::GenericArgsRef<'_>) -> GenericArgs {
    GenericArgs(args.iter().map(|arg| stable(arg.kind())).collect())
}

/// The generic arguments of `impl_def_id` when its `Self` type unifies with `self_ty`, or `None`.
///
/// Fresh inference variables stand in for the impl's generic parameters; unifying the impl's
/// `Self` type with `self_ty` resolves them, so the impl's clauses can be instantiated with the
/// concrete arguments of the current type (e.g. `impl<T: Debug> Trait for Foo<T>` against
/// `Foo<u32>` yields `T = u32`).
fn impl_args_matching<'tcx>(
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    impl_def_id: rustc_span::def_id::DefId,
    self_ty: rustc_middle::ty::Ty<'tcx>,
) -> Option<rustc_middle::ty::GenericArgsRef<'tcx>> {
    use rustc_infer::infer::DefineOpaqueTypes;
    use rustc_middle::traits::ObligationCause;
    use rustc_middle::ty::TypeVisitableExt;
    use rustc_trait_selection::infer::TyCtxtInferExt;
    use rustc_type_ir::TypingMode;

    let infcx = tcx
        .infer_ctxt()
        .with_next_trait_solver(true)
        .ignoring_regions()
        .build(TypingMode::Codegen);
    let fresh = infcx.fresh_args_for_item(DUMMY_SP, impl_def_id);
    let impl_self_ty = tcx
        .type_of(impl_def_id)
        .instantiate(tcx, fresh)
        .skip_normalization();
    let _ = infcx
        .at(&ObligationCause::dummy(), ParamEnv::empty())
        .eq(DefineOpaqueTypes::Yes, impl_self_ty, self_ty)
        .ok()?;
    let resolved = infcx.resolve_vars_if_possible(fresh);
    // An impl whose trait arguments are not determined by its `Self` type (e.g.
    // `impl<T: Marker> Describe<T> for Foo`) leaves `T` unresolved; there is no concrete trait
    // reference to emit for such an impl, so it is skipped.
    if resolved.has_non_region_infer() {
        return None;
    }
    Some(resolved)
}

/// Whether the concrete `trait_ref` holds (modulo regions).
fn type_implements<'tcx>(
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    trait_ref: &rustc_middle::ty::TraitRef<'tcx>,
) -> bool {
    use rustc_trait_selection::infer::InferCtxtExt;
    use rustc_trait_selection::infer::TyCtxtInferExt;
    use rustc_type_ir::TypingMode;

    let infcx = tcx
        .infer_ctxt()
        .with_next_trait_solver(true)
        .ignoring_regions()
        .build(TypingMode::Codegen);
    infcx
        .type_implements_trait(trait_ref.def_id, trait_ref.args.iter(), ParamEnv::empty())
        .must_apply_modulo_regions()
}
