//! Entry point that drives the two analysis passes.
//!
//! [`analyze`] runs:
//! 1. an **HIR pass** ([`visit::HirVisitor`]) over every item-like in the crate, which
//!    handles `const` items (incl. `emit_items`), `ty` aliases and `let` bindings, and
//! 2. a **`rustc_public` pass** ([`CrateAnalysisVisitor`]) over the local crate's ADTs
//!    (including their fields and inherent impl blocks, whose fields and methods are recorded
//!    onto the type), trait declarations (an `emit`-tagged trait and its associated functions)
//!    and free functions.
//!
//! Results are accumulated into a single [`AutozngContext`], from which a `ZngurSpec` is
//! built while `tcx` is alive (see `render.rs`).
//!
//! [`experiment`] is scratch exploration output (trait-detection prototypes); it should be
//! removed or folded into real analysis logic.

use std::ops::ControlFlow;

use itertools::Itertools;
use rustc_middle::ty::TyCtxt;
use rustc_public::{
    CrateDef,
    mir::mono::StaticDef,
    rustc_internal::{internal, stable},
};
use rustc_trait_selection::infer::InferCtxtExt;

use crate::{
    attrs::{AttrCollectionExt, AutozngAttrsExt},
    ctx::AutozngContext,
    driver::{AnalyzeReport, with_ctxt},
    error::span_of_def,
    render,
    rustc_ext::GetAssocInfoExt,
    visit::{self, RustcPublicVisitable, RustcPublicVisitor},
};
use zngur_def::ZngurSpec;

struct CrateAnalysisVisitor<'a> {
    ctx: &'a AutozngContext,
}

impl<'a> RustcPublicVisitor for CrateAnalysisVisitor<'a> {
    type Break = AnalyzeReport;

    fn visit_crate(&mut self, krate: &rustc_public::Crate) -> ControlFlow<Self::Break> {
        let attrs = krate.autozng_attrs();
        attrs.visit(krate, self.ctx);

        krate.super_visit(self)
    }

    fn visit_adt(&mut self, adt: &rustc_public::ty::AdtDef) -> std::ops::ControlFlow<Self::Break> {
        let attrs = adt.autozng_attrs();
        attrs.visit(adt, self.ctx);

        // Children inherit the type's child-targeted attrs (`emit(name)`, `rename_all`).
        // Only walk them when the type itself was emitted: recording a field/method
        // implicitly creates its type's `TyRecord`, so walking children of a `skip`-ed type
        // would leak it into the output.
        if self.ctx.is_ty_recorded(&adt.ty()) {
            for variant in adt.variants() {
                for field in variant.fields() {
                    let field_attrs = field.autozng_attrs();
                    field_attrs.visit(&field, self.ctx);
                }
            }

            // Methods live in the ADT's inherent impl blocks.
            for impl_def in adt.inherent_impls() {
                let impl_attrs = impl_def.autozng_attrs();
                impl_attrs.visit(&impl_def, self.ctx);
            }
        }

        adt.super_visit(self)
    }

    fn visit_trait(
        &mut self,
        trait_def: &rustc_public::ty::TraitDef,
    ) -> std::ops::ControlFlow<Self::Break> {
        let attrs = trait_def.autozng_attrs();
        attrs.visit(trait_def, self.ctx);

        // A trait is only recorded when it carries an explicit `emit` (its implicit action is
        // `None`); when it is, record every associated function. A trait's functions are at most
        // as visible as the trait itself, so there is nothing further to filter. Only descend
        // once the trait is recorded: recording a method implicitly records its trait, so
        // walking a `skip`-ed trait's methods would leak it into the output. A trait recorded as a
        // concrete instantiation (`emit_items(emit_traits = true)`) already has its functions
        // recorded with concrete arguments, so it is not walked again here.
        if self.ctx.is_trait_recorded(trait_def)
            && !self.ctx.is_trait_concretely_recorded(trait_def)
        {
            for assoc_item in trait_def.associated_items() {
                if let Some((fn_def, generics)) = assoc_item.opt_fn_def() {
                    self.ctx.record_trait_method(
                        assoc_item.opt_trait_def().unwrap(),
                        assoc_item.opt_trait_ref(),
                        fn_def,
                        generics,
                        span_of_def(fn_def.def_id()),
                    );
                }
            }
        }

        trait_def.super_visit(self)
    }

    fn visit_impl(
        &mut self,
        impl_def: &rustc_public::ty::ImplDef,
    ) -> std::ops::ControlFlow<Self::Break> {
        // Trait impls are visited here (inherent impls are walked from `visit_adt`); only an
        // `#[autozng::emit_on_implementers]` impl does anything (see `EmitOnImplTarget`).
        let attrs = impl_def.autozng_attrs();
        attrs.visit(impl_def, self.ctx);
        impl_def.super_visit(self)
    }

    fn visit_fn(&mut self, fn_def: &rustc_public::ty::FnDef) -> std::ops::ControlFlow<Self::Break> {
        // Associated functions (methods) are handled through their impl block in `visit_adt`;
        // only free functions are recorded here.
        if fn_def.associated_item().is_none() {
            let attrs = fn_def.autozng_attrs();
            attrs.visit(fn_def, self.ctx);
        }
        fn_def.super_visit(self)
    }

    fn visit_static(&mut self, st: &StaticDef) -> ControlFlow<Self::Break> {
        st.super_visit(self)
    }
}

pub fn analyze(tcx: TyCtxt) -> std::ops::ControlFlow<AnalyzeReport, ZngurSpec> {
    // NOTE: the context borrows `tcx`, so the `ZngurSpec` is built from it before this
    // function returns, while the `TyCtxt` is still alive. Printing happens later (see
    // `render` / `driver`).
    let mut ctx = AutozngContext::new(with_ctxt(|driver| driver.cfg.clone()));

    let krate = rustc_public::local_crate();
    let crate_name = &krate.name;
    eprintln!("--- Analyzing crate: {crate_name}");

    tcx.hir_visit_all_item_likes_in_crate(&mut visit::HirVisitor { ctx: &mut ctx, tcx });
    CrateAnalysisVisitor { ctx: &ctx }.visit_all_rustc_public_items()?;

    // eprintln!("AFTER VISIT CTX: {ctx:#?}");

    // experiment(tcx);

    if tcx.dcx().has_errors().is_some() {
        ControlFlow::Break(AnalyzeReport {
            errors: tcx.dcx().err_count(),
        })
    } else {
        // Build the owned spec while `tcx` is alive; the driver prints it after
        // `run_compiler` returns (see `render::build_spec` / `render::print_to_file`).
        ControlFlow::Continue(render::build_spec(&ctx))
    }
}

/// Scratch/exploration code: prototypes for detecting which traits implementers should be
/// emitted for and inspecting trait associated fns. Not part of the real pipeline.
#[allow(dead_code)] // scratch exploration kept for reference
fn experiment(tcx: TyCtxt) {
    eprintln!("===========================");
    eprintln!("     Trait searching ");
    eprintln!("===========================");
    let mut rustc_traits = Vec::new();
    let mut rustc_public_traits = Vec::new();
    tcx.all_traits_including_private().for_each(|trait_id| {
        rustc_traits.push(stable(trait_id).name());
    });
    rustc_public::all_trait_decls()
        .iter()
        .for_each(|trait_def| {
            rustc_public_traits.push(trait_def.name());
        });
    rustc_public_traits.sort();
    rustc_traits.sort();
    let inner_count = rustc_traits.len();
    let public_count = rustc_public_traits.len();
    eprintln!(
        "inner count {inner_count} == public count {public_count} => {}",
        inner_count == public_count
    );
    rustc_traits
        .iter()
        .zip(&rustc_public_traits)
        .for_each(|(inner, public)| {
            if inner != public {
                eprintln!("inner {inner} != public {public}");
            }
        });

    let mut emit_for_traits = Vec::new();

    rustc_public::all_trait_decls()
        .into_iter()
        .for_each(|trait_def| {
            let attrs = trait_def.autozng_attrs();
            if !attrs.is_empty() {
                eprintln!("trait {} has attrs {:?}", trait_def.name(), attrs);
                trait_def.associated_items().iter().for_each(|assoc_item| {
                    if let Some(assoc_item_sig) = assoc_item.opt_fn_sig() {
                        eprintln!(
                            "    fn {}({}) -> {}",
                            assoc_item.def_id.name(),
                            assoc_item_sig.inputs().iter().join(", "),
                            assoc_item_sig.output()
                        );
                    }
                });
                emit_for_traits.push(trait_def);
            }
        });
    eprintln!("===========================");
    eprintln!("     Trait searching END ");
    eprintln!("===========================");

    // eprintln!("===========================");
    // eprintln!("     CrateItem Iter Const test ");
    // eprintln!("===========================");

    // let hir_const_body = |item: &CrateItem| -> Option<&rustc_hir::Body> {
    //     let rhs = match tcx.hir_node_by_def_id(internal(tcx, item).as_local().unwrap()) {
    //         rustc_hir::Node::Item(item) => match item.kind {
    //             rustc_hir::ItemKind::Const(_ident, _generics, _ty, rhs) => Some(rhs),
    //             _ => None,
    //         },
    //         rustc_hir::Node::ImplItem(item) => match item.kind {
    //             rustc_hir::ImplItemKind::Const(_ty, rhs) => Some(rhs),
    //             _ => None,
    //         },
    //         _ => None,
    //     }?;
    //     match rhs {
    //         rustc_hir::ConstItemRhs::Body(body_id) => Some(tcx.hir_body(body_id)),
    //         _ => None,
    //     }
    // };

    // rustc_public::all_local_items()
    //     .into_iter()
    //     .for_each(|crate_item| {
    //         let item_kind = crate_item.kind();
    //         eprintln!("{crate_item:?}  => {item_kind:?} ;");
    //         if let rustc_public::ItemKind::Const = item_kind
    //             && let Some(const_body) = hir_const_body(&crate_item)
    //         {
    //             eprintln!("{{");
    //             if let rustc_hir::Expr {
    //                 kind:
    //                     rustc_hir::ExprKind::Block(
    //                         rustc_hir::Block {
    //                             stmts, expr: None, ..
    //                         },
    //                         _,
    //                     ),
    //                 ..
    //             } = *const_body.value {
    //                 stmts.iter().for_each(|stmt| {
    //                     if let rustc_hir::StmtKind::Let(rustc_hir::LetStmt {
    //                         pat:
    //                             rustc_hir::Pat {
    //                                 kind: rustc_hir::PatKind::Wild | rustc_hir::PatKind::Binding(..),
    //                                 ..
    //                             },
    //                         init:
    //                             Some(rustc_hir::Expr {
    //                                 hir_id: stmt_expr_id,
    //                                 ..
    //                             }),
    //                         hir_id: _stmt_id,
    //                         ..
    //                     }) = stmt.kind
    //                     {
    //                         let typeck_results = tcx.typeck(stmt.hir_id.owner.def_id);
    //                         let node_ty = stable(typeck_results.node_type(*stmt_expr_id));

    //                         if let rustc_public::ty::TyKind::RigidTy(
    //                             rustc_public::ty::RigidTy::FnDef(fn_def, generics),
    //                         ) = node_ty.kind()
    //                         {
    //                             let attrs = stmt.autozng_attrs();
    //                             eprintln!("     DOCS/TOOLS: {attrs:?}");
    //                             let fn_generics: Vec<_> = fn_def.generics_of().params;
    //                             let generics_clause = if fn_generics.is_empty() {
    //                                 "".to_owned()
    //                             } else {
    //                                 format!(
    //                                     "<{}>",
    //                                     fn_generics
    //                                         .iter()
    //                                         .map(|generic_def| {
    //                                             let arg_name = &generic_def.name;
    //                                             match generics
    //                                                 .0
    //                                                 .get(generic_def.index as usize)
    //                                                 .and_then(|arg| {
    //                                                     match generic_def.kind {
    //                                                         rustc_public::ty::GenericParamDefKind::Type { .. } => Some(arg.expect_ty().to_string()),
    //                                                         rustc_public::ty::GenericParamDefKind::Const { .. } => Some(match arg.expect_const().kind() {
    //                                                             rustc_public::ty::TyConstKind::Value(const_arg_ty, alloc) => match const_arg_ty.kind() {
    //                                                                 TyKind::RigidTy(RigidTy::Uint(_)) => alloc.read_uint().unwrap().to_string(),
    //                                                                 TyKind::RigidTy(RigidTy::Int(_)) => alloc.read_int().unwrap().to_string(),
    //                                                                 TyKind::RigidTy(RigidTy::Bool) => alloc.read_bool().unwrap().to_string(),
    //                                                                 _ => const_arg_ty.to_string()
    //                                                             },
    //                                                             const_arg => format!("{const_arg:?}"),
    //                                                         }),
    //                                                         rustc_public::ty::GenericParamDefKind::Lifetime => None
    //                                                     }
    //                                                 })
    //                                             {
    //                                                 Some(g_ty) => {
    //                                                     g_ty
    //                                                 }
    //                                                 None => arg_name.to_string(),
    //                                             }
    //                                         })
    //                                         .join(", ")
    //                                 )
    //                             };
    //                             let generics_list = generics
    //                                 .0
    //                                 .iter()
    //                                 .filter_map(|arg| arg.ty().map(ToString::to_string))
    //                                 .join(", ");
    //                             eprintln!(
    //                                 "    use {}{generics_clause} with ({generics_list})",
    //                                 fn_def.name()
    //                             );
    //                         }
    //                     }
    //                 });
    //             } else {
    //                 let const_ty = crate_item.ty();
    //                 let attrs = crate_item.autozng_attrs();
    //                 eprintln!("     DOCS/TOOLS: {attrs:?}");
    //                 eprintln!(
    //                     "Const Ty: {const_ty}"
    //                 );
    //             }

    //             eprintln!("}}");
    //         }
    //     });
    // eprintln!("===========================");
    // eprintln!("     CrateItem Iter test END ");
    // eprintln!("===========================");

    eprintln!("===========================");
    eprintln!("     Crate Iter trait detection test ");
    eprintln!("===========================");

    let krate = rustc_public::local_crate();
    krate.adts().into_iter().for_each(|adt| {
        let adt_def_id = internal(tcx, adt.def_id());
        let adt_ty = internal(tcx, adt.ty());

        emit_for_traits.iter().for_each(|trait_def| {
            let trait_def_id = internal(tcx, trait_def.def_id());

            use rustc_trait_selection::infer::TyCtxtInferExt;

            use rustc_type_ir::TypingMode;

            let infcx = tcx
                .infer_ctxt()
                .with_next_trait_solver(true)
                .ignoring_regions()
                .build(TypingMode::Codegen);

            let result =
                infcx.type_implements_trait(trait_def_id, [adt_ty], tcx.param_env(adt_def_id));

            eprintln!(
                "{} implements {} ? {}",
                adt.name(),
                trait_def.name(),
                result.must_apply_modulo_regions()
            );
        });
    });

    eprintln!("===========================");
    eprintln!("     Crate Iter trait detection test END ");
    eprintln!("===========================");
}
