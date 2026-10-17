use rustc_attr_parsing::template;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrBuilder, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttrtibuteParser},
        },
    },
    ctx::AutozngContext,
};

pub trait EmitItemsTarget {
    fn emit(&self, _ctx: &AutozngContext) {}
}

#[derive(Debug)]
pub struct EmitItems;

const EMIT_ITEMS_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "emit_items"]);

impl EmitItems {
    // fn visit_hir_const_item<'tcx>(
    //     &self,
    //     _ctx: &mut AutozngContext,
    //     const_ty: &rustc_hir::Ty<'tcx>,
    //     const_body: &rustc_hir::Body,
    //     _impl_item_id: Option<rustc_span::def_id::DefId>,
    // ) {
    //     with_tcx(|tcx| {
    //         let ty = rustc_hir_analysis::lower_ty(tcx, const_ty);
    //         eprintln!(" found Anon const! with body: type = {ty}");
    //         if ty == rustc_middle::ty::Ty::new_tup(tcx, &[]) {
    //             eprintln!("    type is Unit");
    //             if let rustc_hir::Expr {
    //                 kind:
    //                     rustc_hir::ExprKind::Block(
    //                         rustc_hir::Block {
    //                             stmts, expr: None, ..
    //                         },
    //                         _,
    //                     ),
    //                 ..
    //             } = *const_body.value
    //             {
    //                 for stmt in *stmts {
    //                     if let rustc_hir::StmtKind::Let(rustc_hir::LetStmt {
    //                         pat:
    //                             rustc_hir::Pat {
    //                                 kind: rustc_hir::PatKind::Wild | rustc_hir::PatKind::Binding(..),
    //                                 ..
    //                             },
    //                         init:
    //                             Some(rustc_hir::Expr {
    //                                 hir_id: stmt_expr_id,
    //                                 kind: rustc_hir::ExprKind::Path(_stmt_rhs_path),
    //                                 ..
    //                             }),
    //                         ..
    //                     }) = stmt.kind
    //                     {
    //                         let attrs = stmt.autozng_attrs();
    //                         eprintln!("      ATTRS: {attrs:?}");
    //                         let typeck_results = tcx.typeck(stmt.hir_id.owner.def_id);
    //                         let node_ty = typeck_results.node_type(*stmt_expr_id);

    //                         let ty_kind = node_ty.kind();
    //                         if let rustc_middle::ty::TyKind::FnDef(fn_def_id, fn_generics) =
    //                             *ty_kind
    //                         {
    //                             let mut fn_generics: SmallVec<[GenericArg; 8]> = tcx
    //                                 .instantiate_bound_regions_with_erased(fn_generics)
    //                                 .to_smallvec();
    //                             fn_generics.iter_mut().for_each(|arg| {
    //                             if let GenericArgKind::Type(generic_ty) = arg.kind()
    //                                 && let TyKind::Param(generic_ty_param) = generic_ty.kind()
    //                             {
    //                                 eprintln!(
    //                                     "        replacing unbound param {} `{}` with `u8` for testing",
    //                                     generic_ty_param.index, generic_ty_param.name
    //                                 );
    //                                 *arg = Ty::new_uint(tcx, rustc_middle::ty::UintTy::U8).into();
    //                             }
    //                         });
    //                             let (parent_name, on_ty) = {
    //                                 let (parent_id, def_kind) =
    //                                     tcx.assoc_parent(fn_def_id).unwrap();
    //                                 if let DefKind::Trait = def_kind {
    //                                     let trait_def = tcx.trait_def(parent_id);
    //                                     (
    //                                         tcx.item_name(trait_def.def_id).to_string(),
    //                                         Some(fn_generics.first().unwrap().as_type().unwrap()),
    //                                     )
    //                                 } else {
    //                                     let impl_ty = tcx
    //                                         .type_of(parent_id)
    //                                         .instantiate_identity()
    //                                         .skip_normalization();
    //                                     (impl_ty.to_string(), None)
    //                                 }
    //                             };
    //                             let fn_name = tcx.item_name(fn_def_id).to_string();
    //                             let sig = tcx.instantiate_bound_regions_with_erased(
    //                                 tcx.fn_sig(fn_def_id)
    //                                     .instantiate(tcx, fn_generics.as_slice())
    //                                     .skip_normalization(),
    //                             );
    //                             eprintln!(
    //                                 "    fn to mono : fn {parent_name}::{fn_name}({}) -> {}{};",
    //                                 sig.inputs()
    //                                     .iter()
    //                                     .map(|input| input.to_string())
    //                                     .join(", "),
    //                                 sig.output(),
    //                                 on_ty
    //                                     .map(|on_ty| format!(" on {on_ty}"))
    //                                     .unwrap_or("".to_string())
    //                             );
    //                         } else {
    //                             eprintln!("   node_ty: {node_ty:?}");
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     })
    // }
}

impl Attr for EmitItems {
    type Target = dyn EmitItemsTarget;
    fn path(&self) -> AttrPath {
        EMIT_ITEMS_PATH
    }

    fn visit(&self, item: &dyn EmitItemsTarget, ctx: &AutozngContext) {
        item.emit(ctx);
    }
}

#[derive(Debug, Default)]
pub struct EmitItemsParser {
    valid: bool,
}

impl EmitItemsParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        self.valid = cx.expect_no_args(args).is_some();
    }
}

impl AttrtibuteParser for EmitItemsParser {
    type Attr = EmitItems;
    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        EMIT_ITEMS_PATH,
        template!(Word, "https://link/to/docs"),
        EmitItemsParser::parse,
    )];

    fn finalize(self, target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        self.valid.then_some(EmitItems)
    }
}
