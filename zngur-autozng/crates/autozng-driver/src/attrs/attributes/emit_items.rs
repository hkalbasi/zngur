use rustc_attr_parsing::template;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttributeParser},
        },
    },
    ctx::AutozngContext,
};

pub trait EmitItemsTarget {
    /// Process the `const` tagged `#[autozng::emit_items]`.
    ///
    /// `emit_traits` is the parsed value of the attribute's `emit_traits = true|false` argument
    /// (default `false`): when set, the enumerated items are treated as concrete *trait* export
    /// targets (`dyn Trait<ConcreteArgs>`) rather than as types.
    fn emit(&self, _emit_traits: bool, _ctx: &AutozngContext) {}
}

/// `#[autozng::emit_items]` / `#[autozng::emit_items(emit_traits = true)]` on a `const`.
///
/// The `const` item (through [`EmitItemsTarget`]) is the "attributed item": its own span is what
/// the enumerated items record, so the marker carries no span of its own.
#[derive(Debug, Clone)]
pub struct EmitItems {
    /// `emit_traits = true` was given: enumerated `dyn Trait<ConcreteArgs>` items name concrete
    /// trait instantiations to export (a generic trait is only representable in `.zng` once all
    /// its type parameters are known). Defaults to `false`.
    pub emit_traits: bool,
}

const EMIT_ITEMS_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "emit_items"]);

const EMIT_TRAITS_KEY: &str = "emit_traits";

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
        item.emit(self.emit_traits, ctx);
    }
}

#[derive(Debug, Default)]
pub struct EmitItemsParser {
    valid: bool,
    emit_traits: bool,
}

impl EmitItemsParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        match args {
            ArgParser::NoArgs => self.valid = true,
            ArgParser::List(list) => {
                let mut valid = true;
                let mut seen_emit_traits = false;
                for arg in list.mixed() {
                    let Some((key, value)) =
                        cx.expect_name_value(arg, arg.span(), Some(EMIT_TRAITS_KEY))
                    else {
                        valid = false;
                        continue;
                    };
                    if key.value != EMIT_TRAITS_KEY {
                        cx.expected_specific_argument(key.span, &[EMIT_TRAITS_KEY]);
                        valid = false;
                        continue;
                    }
                    if seen_emit_traits {
                        cx.duplicate_key(key.span, EMIT_TRAITS_KEY);
                        valid = false;
                        continue;
                    }
                    seen_emit_traits = true;

                    let Some(value) = cx.expect_bool_literal(value) else {
                        valid = false;
                        continue;
                    };
                    self.emit_traits = value.value;
                }
                self.valid = valid;
            }
            // `#[autozng::emit_items = true]` is not a valid form.
            ArgParser::NameValue(_) => {
                cx.expected_list_or_no_args(cx.span);
            }
        }
    }
}

impl AttributeParser for EmitItemsParser {
    type Attr = EmitItems;
    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        EMIT_ITEMS_PATH,
        template!(
            Word,
            List: &["emit_traits = true", "emit_traits = false"],
            "https://link/to/docs"
        ),
        EmitItemsParser::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        self.valid.then_some(EmitItems {
            emit_traits: self.emit_traits,
        })
    }
}
