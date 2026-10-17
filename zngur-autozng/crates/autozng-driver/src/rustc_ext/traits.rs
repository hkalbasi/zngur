use rustc_middle::ty::ParamEnv;
use rustc_public::{
    CrateDef,
    rustc_internal::stable,
    ty::{GenericArgs, TraitRef, Ty},
};
use rustc_span::DUMMY_SP;

use crate::{driver::with_ctxt, rustc_ext::RustcInternalEx};

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
    pub fn trait_decl(&self) -> rustc_public::ty::TraitDecl {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            let def_id = match self {
                Self::Debug => tcx
                    .all_traits_including_private()
                    .find(|def_id| tcx.def_path_str(*def_id) == "std::fmt::Debug")
                    .expect("std::fmt::Debug is not defined"),
                Self::Copy => tcx.require_lang_item(rustc_attr_ir::LangItem::Copy, DUMMY_SP),
                Self::Unsized => tcx.require_lang_item(rustc_attr_ir::LangItem::Sized, DUMMY_SP),
                Self::PartialEq => {
                    tcx.require_lang_item(rustc_attr_ir::LangItem::PartialEq, DUMMY_SP)
                }
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
        })
    }

    pub fn must_impl_trait(&self) -> bool {
        !matches!(self, Self::Unsized)
    }

    pub fn need_generic(&self) -> bool {
        match self {
            Self::PartialEq => true,
            Self::PartialOrd => true,
            Self::Add => true,
            Self::Sub => true,
            Self::Mul => true,
            Self::Div => true,
            Self::Rem => true,
            Self::BitAnd => true,
            Self::BitOr => true,
            Self::BitXor => true,
            Self::Shl => true,
            Self::Shr => true,
            Self::Drop => true,
            Self::IntoIterator => true,
            _ => false,
        }
    }
}

pub trait WellKownTraitsExt {
    fn implimented_wellknown_traits(&self) -> Vec<(WellKnownTrait, TraitRef)>;
}

impl WellKownTraitsExt for Ty {
    fn implimented_wellknown_traits(&self) -> Vec<(WellKnownTrait, TraitRef)> {
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
            let generic_wellknown = WELLKNOWNTRAITS
                .iter()
                .copied()
                .filter(|trt| trt.need_generic())
                .collect::<Vec<_>>();
            results.extend(
                rustc_public::all_trait_impls()
                    .into_iter()
                    .map(|impl_def| {
                        let trt_ref = impl_def.trait_impl().value;
                        generic_wellknown
                            .iter()
                            .filter_map(|trt_enm| {
                                let known_trt_decl = trt_enm.trait_decl();
                                if known_trt_decl.def_id.def_id() == trt_ref.def_id.def_id()
                                    && trt_ref.self_ty() == *self
                                {
                                    Some((*trt_enm, trt_ref.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .flatten(),
            );
            results
        })
    }
}
