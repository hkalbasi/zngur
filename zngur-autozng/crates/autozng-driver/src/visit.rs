//! The two crate-walking visitors plus the `rustc_public` visit trait they share.
//!
//! - [`HirVisitor`] walks the HIR to find `const`/`ty`/`let` items (things the
//!   `rustc_public` API exposes poorly).
//! - [`CrateAnalysisVisitor`](crate::analyze) (defined in `analyze.rs`) walks the local
//!   crate's ADTs, traits and trait impls via [`RustcPublicVisitor`].
//!
//! Trait handling is **stubbed**: the `TraitDef`/`ImplDef` `super_visit` impls below are
//! no-ops, and `CrateAnalysisVisitor` has no `visit_trait`/`visit_impl`, so traits and
//! impls are currently not analyzed. The large commented block in the `AdtDef` impl is an
//! earlier sketch of the intended field/method/well-known-trait emission.

use rustc_middle::ty::TyCtxt;

use std::ops::ControlFlow;

use rustc_public::{
    Crate, CrateDef,
    mir::mono::StaticDef,
    ty::{AdtDef, FnDef},
};

use crate::{
    attrs::{AttrCollectionExt, AutozngAttrsExt},
    ctx::AutozngContext,
    rustc_ext::{HirConstBodyExt, HirItemExt},
};

pub struct HirVisitor<'ctx, 'tcx> {
    pub ctx: &'ctx mut AutozngContext,
    pub tcx: TyCtxt<'tcx>,
}

pub struct OnlyItems();

impl<'tcx> rustc_hir::intravisit::NestedFilter<'tcx> for OnlyItems {
    type MaybeTyCtxt = TyCtxt<'tcx>;

    const INTER: bool = true;
    const INTRA: bool = false;
}

impl<'tcx> rustc_hir::intravisit::Visitor<'tcx> for HirVisitor<'_, 'tcx> {
    type NestedFilter = OnlyItems;

    /// allow nested traversal
    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_impl_item(&mut self, ii: &'tcx rustc_hir::ImplItem<'tcx>) -> Self::Result {
        if let Some(const_item) = ii.opt_const_item() {
            let attrs = const_item.autozng_attrs();
            let _results = attrs.visit(&const_item, self.ctx);
        }
    }

    fn visit_item(&mut self, i: &'tcx rustc_hir::Item<'tcx>) -> Self::Result {
        if let Some(const_item) = i.opt_const_item() {
            let attrs = const_item.autozng_attrs();
            let _ = attrs.visit(&const_item, self.ctx);
        } else if let Some(alias_item) = i.opt_ty_alias() {
            let attrs = alias_item.autozng_attrs();
            let _ = attrs.visit(&alias_item, self.ctx);
        }
    }
}

pub trait RustcPublicVisitor: Sized {
    type Break;

    fn visit_crate(&mut self, krate: &Crate) -> ControlFlow<Self::Break> {
        krate.super_visit(self)
    }

    fn visit_adt(&mut self, adt: &AdtDef) -> ControlFlow<Self::Break> {
        adt.super_visit(self)
    }

    fn visit_static(&mut self, st: &StaticDef) -> ControlFlow<Self::Break> {
        st.super_visit(self)
    }

    fn visit_fn(&mut self, fn_def: &FnDef) -> ControlFlow<Self::Break> {
        fn_def.super_visit(self)
    }

    fn visit_all_rustc_public_items(&mut self) -> ControlFlow<Self::Break> {
        // visit all items except `CrateItems` from  `rustc_public::all_local_items()`
        // that interface is cumbersome, and the work is mostly duplicated by visiting the local `Crate`
        // visiting aliases and `const` items is better done via traversing the HIR anyway

        rustc_public::local_crate().visit(self)?;

        rustc_public::all_trait_decls().visit(self)?;
        rustc_public::all_trait_impls().visit(self)
    }
}

pub trait RustcPublicVisitable {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.super_visit(visitor)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break>;
}

impl<T: RustcPublicVisitable> RustcPublicVisitable for Vec<T> {
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for arg in self {
            arg.visit(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::Crate {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_crate(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        // visit statics (probably never used but the functionality is here)
        for st in self.statics() {
            st.visit(visitor)?;
        }
        // visit adts
        for adt in self.adts() {
            adt.visit(visitor)?;
        }
        // visit fn defs
        for fn_def in self.fn_defs() {
            fn_def.visit(visitor)?;
        }

        // visiting traits may visit come items twice is using `visit_all_rustc_public_items`
        // so be careful not to duplicate things
        for trait_def in self.trait_decls() {
            trait_def.visit(visitor)?;
        }
        for trait_impl in self.trait_impls() {
            trait_impl.visit(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::mir::mono::StaticDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        eprintln!("Visiting Static: {}", self.name());
        eprintln!("  Type: {}", self.ty());
        visitor.visit_static(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::FnDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_fn(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::AdtDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_adt(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
        // // let self_emit_action = attrs.get_emit_action(&EmitItem::This);
        // if self_emit_action.emit_for_visibility(vis) {
        //     let path = name
        //         .split(PATH_SEP)
        //         .map(Into::into)
        //         .collect::<Vec<String>>();
        //     let layout = self
        //         .ty()
        //         .layout()
        //         .map(|layout| {
        //             let shape = layout.shape();
        //             LayoutPolicy::StackAllocated {
        //                 size: shape.size.bytes(),
        //                 align: shape.abi_align as usize,
        //             }
        //         })
        //         .ok();
        //     eprintln!("Generics of {}: {:?}", name, generics);
        //     let adt_clauses = self.clauses_of();
        //     eprintln!("    where {adt_clauses:?}");

        //     let ty = RustType::Adt(RustPathAndGenerics {
        //         path,
        //         generics: Vec::new(),
        //         named_generics: Vec::new(),
        //     });

        //     eprintln!("Trait Impls of {}", name);
        //     let traits = self.ty().implimented_wellknown_traits();
        //     let wellknown_traits = traits
        //         .iter()
        //         .filter_map(|(trt_enm, trt_ref)| {
        //             eprintln!(
        //                 "  impl {}{} for {}",
        //                 if trt_enm.must_impl_trait() { "" } else { "?" },
        //                 trt_ref.def_id.name(),
        //                 name
        //             );
        //             match trt_enm {
        //                 WellKnownTrait::Debug => Some(ZngurWellknownTrait::Debug),
        //                 WellKnownTrait::Copy => Some(ZngurWellknownTrait::Copy),
        //                 WellKnownTrait::Unsized => Some(ZngurWellknownTrait::Unsized),
        //                 WellKnownTrait::Drop => Some(ZngurWellknownTrait::Drop),
        //                 _ => None,
        //             }
        //         })
        //         .collect();

        //     let fields: Vec<ZngurField> = self
        //         .ty()
        //         .layout()
        //         .ok()
        //         .and_then(|layout| match (self.kind(), layout.shape().fields) {
        //             (AdtKind::Struct, FieldsShape::Arbitrary { offsets }) => Some(
        //                 self.variants()
        //                     .pop()
        //                     .unwrap()
        //                     .fields()
        //                     .into_iter()
        //                     .zip(offsets)
        //                     .filter_map(|(field, offset)| {
        //                         match ctx.record_ty_dependency(field.ty()) {
        //                             Ok(ty) => Some(ZngurField {
        //                                 ty,
        //                                 name: field.name,
        //                                 offset: Some(offset.bytes()),
        //                             }),
        //                             Err(msg) => {
        //                                 emit_warning_for_rustc_public_span(field.span(), msg);
        //                                 None
        //                             }
        //                         }
        //                     })
        //                     .collect(),
        //             ),
        //             _ => None,
        //         })
        //         .unwrap_or_default();

        //     fields.iter().for_each(|f| {
        //         eprintln!("    FIELD {}: {} @ {:?}", f.name, f.ty, f.offset);
        //     });

        //     let inherent_impls = self.inherent_impls();

        //     let mut methods = Vec::new();

        //     // let attrs_for_children = attrs.for_child();

        //     for impl_def in inherent_impls {
        //         // let impl_span = impl_def.span();
        //         let impl_generics = impl_def.generics_of();
        //         eprintln!(
        //             "Impl of {} with generics {:?}",
        //             impl_def.0.name(),
        //             impl_generics
        //         );

        //         let clauses = impl_def.clauses_of();
        //         eprintln!("where {clauses:?}");

        //         for assoc_item in impl_def.associated_items() {
        //             let assoc_item_span = assoc_item.def_id.span();
        //             if let AssocKind::Fn {
        //                 name: assoc_item_name,
        //                 has_self,
        //             } = &assoc_item.kind
        //             {
        //                 let assoc_item_vis = assoc_item.visibility();
        //                 let fn_sig = assoc_item.opt_fn_sig().unwrap();
        //                 let assoc_item_generics = fn_sig
        //                     .inputs_and_output
        //                     .iter()
        //                     .filter(|var| {
        //                         matches!(var.kind(), TyKind::Param(_) | TyKind::Bound(_, _))
        //                     })
        //                     .collect::<Vec<_>>();

        //                 let self_ty = self.ty();

        //                 eprintln!(
        //                     "{} fn {}({}) -> {}",
        //                     assoc_item_vis,
        //                     assoc_item_name,
        //                     fn_sig
        //                         .inputs()
        //                         .iter()
        //                         .enumerate()
        //                         .map(|(i, t)| {
        //                             if i == 0 && *has_self {
        //                                 match t.kind() {
        //                                     TyKind::RigidTy(RigidTy::Ref(
        //                                         _,
        //                                         fn_first_ty,
        //                                         mutability,
        //                                     )) if fn_first_ty == self_ty => {
        //                                         format!(
        //                                             "&{}{}",
        //                                             match mutability {
        //                                                 Mutability::Mut => "mut ",
        //                                                 Mutability::Not => "",
        //                                             },
        //                                             "self"
        //                                         )
        //                                     }
        //                                     TyKind::RigidTy(RigidTy::Adt(fn_first_ty_adt, _))
        //                                         if fn_first_ty_adt.ty() == self_ty =>
        //                                     {
        //                                         "self".to_string()
        //                                     }
        //                                     _ => format!("self: {t}"),
        //                                 }
        //                             } else {
        //                                 t.to_string()
        //                             }
        //                         })
        //                         .join(", "),
        //                     fn_sig.output(),
        //                 );

        //                 let assoc_item_attrs = assoc_item.autozng_attrs();
        //                 let assoc_item_emit_action = assoc_item_attrs
        //                     .get_emit_action(&EmitItem::This)
        //                     | attrs_for_children
        //                         .get_emit_action(&EmitItem::Name(vec![assoc_item_name.clone()]));
        //                 if assoc_item_emit_action.emit_for_visibility(assoc_item_vis)
        //                     && assoc_item_generics.is_empty()
        //                     && let Some((inputs, output)) = {
        //                         let (inputs, mut errors): (Vec<_>, Vec<_>) = fn_sig
        //                             .inputs()
        //                             .iter()
        //                             .copied()
        //                             .enumerate()
        //                             .map(|(index, arg): (usize, Ty)| {
        //                                 with_tcx(|tcx| {
        //                                     map_rustc_public_ty_to_zngur(arg, tcx)
        //                                         .map_err(|err| (err, index))
        //                                 })
        //                             })
        //                             .partition_result();
        //                         if errors.is_empty() {
        //                             match with_tcx(|tcx| {
        //                                 map_rustc_public_ty_to_zngur(fn_sig.output(), tcx)
        //                             }) {
        //                                 Ok(output) => Some((inputs, output)),
        //                                 Err(err) => {
        //                                     with_struct_warning_for_rustc_public_span(
        //                                         assoc_item_span,
        //                                         "there was an error processing the function output type and it will not be emitted".to_string(),
        //                                         |mut warn_diag, tcx| {

        //                                     if let Some(local_id) =
        //                                         assoc_item.def_id.def_id().internal(tcx).as_local()
        //                                         && let Some(assoc_decl) =
        //                                             tcx.hir_fn_decl_by_hir_id(
        //                                                 tcx.local_def_id_to_hir_id(local_id),
        //                                             )
        //                                     {
        //                                         warn_diag.span_note(assoc_decl.output.span(), err);
        //                                     } else {
        //                                         warn_diag.note(err);
        //                                     }

        //                                     warn_diag.emit();
        //                                         }
        //                                     );

        //                                     None
        //                                 }
        //                             }
        //                         } else {
        //                             errors.sort_by_key(|(_, a)| *a);
        //                             with_struct_warning_for_rustc_public_span(
        //                                 assoc_item_span,
        //                                 "there was an error processing the function input types and it will not be emitted".to_string(),
        //                                 |mut warn_diag, tcx|{

        //                             if let Some(local_id) = assoc_item.def_id.def_id().internal(tcx).as_local()
        //                                 && let Some(assoc_body) =
        //                                     tcx.hir_maybe_body_owned_by(local_id)
        //                             {
        //                                 let error_indexes: Vec<_> = errors
        //                                     .iter()
        //                                     .map(|(_, index)| index)
        //                                     .copied()
        //                                     .collect();
        //                                 for (param, (msg, _)) in assoc_body
        //                                     .params
        //                                     .iter()
        //                                     .enumerate()
        //                                     .filter_map(|(index, param)| {
        //                                         if error_indexes.contains(&index) {
        //                                             Some(param)
        //                                         } else {
        //                                             None
        //                                         }
        //                                     })
        //                                     .zip(errors)
        //                                 {
        //                                     warn_diag.span_note(param.span, msg);
        //                                 }
        //                             } else {
        //                                 for (msg, param_index) in errors {
        //                                     warn_diag
        //                                         .note(format!("for param {param_index}: {msg}"));
        //                                 }
        //                             }
        //                             warn_diag.emit();
        //                             });
        //                             None
        //                         }
        //                     }
        //                 {
        //                     methods.push(ZngurMethodDetails {
        //                         data: ZngurMethod {
        //                             name: assoc_item_name.clone(),
        //                             generics: Vec::new(),
        //                             receiver: if *has_self {
        //                                 match fn_sig.inputs().first().unwrap().kind() {
        //                                     TyKind::RigidTy(RigidTy::Ref(
        //                                         _,
        //                                         fn_first_ty,
        //                                         mutability,
        //                                     )) if fn_first_ty == self_ty => {
        //                                         ZngurMethodReceiver::Ref(match mutability {
        //                                             Mutability::Mut => zngur_def::Mutability::Mut,
        //                                             Mutability::Not => zngur_def::Mutability::Not,
        //                                         })
        //                                     }
        //                                     TyKind::RigidTy(RigidTy::Adt(fn_first_ty_adt, _))
        //                                         if fn_first_ty_adt.ty() == self_ty =>
        //                                     {
        //                                         ZngurMethodReceiver::Move
        //                                     }
        //                                     _ => ZngurMethodReceiver::Static,
        //                                 }
        //                             } else {
        //                                 ZngurMethodReceiver::Static
        //                             },
        //                             inputs,
        //                             output,
        //                             is_safe: matches!(
        //                                 fn_sig.safety,
        //                                 rustc_public::mir::Safety::Safe
        //                             ),
        //                         },
        //                         use_path: None,
        //                         deref: None,
        //                         cpp_name: None,
        //                     });
        //                 }
        //             }
        //         }
        //     }

        //     let zng_ty = ZngurType {
        //         ty,
        //         layout,
        //         wellknown_traits,
        //         methods,
        //         constructors: Vec::new(),
        //         fields,
        //         cpp_value: None,
        //         cpp_ref: None,
        //         cpp_stack_owned: None,
        //     };
        //     VisitResult::NewItem {
        //         path: self.name(),
        //         item: ZngurAstItem::Type(zng_ty),
        //     }
        // } else {
        //     VisitResult::NoOp
        // }
    }
}

impl RustcPublicVisitable for rustc_public::ty::TraitDef {
    // TODO: stubbed — trait decls are visited but produce nothing.
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::ImplDef {
    // TODO: stubbed — trait impls are visited but produce nothing.
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}
