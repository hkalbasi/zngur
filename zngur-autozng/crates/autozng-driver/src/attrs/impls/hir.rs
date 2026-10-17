//! implementations for rustc_hir types / crate::rustc_ext::hir wrappers

use rustc_public::rustc_internal::stable;

use crate::{
    attrs::{
        AttrCollectionExt, AutozngAttrsExt,
        attributes::{
            emit::{EmitAction, EmitTarget},
            emit_items::EmitItemsTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
};

impl EmitItemsTarget for crate::rustc_ext::HirConstItem {
    fn emit(&self, ctx: &AutozngContext) {
        // record any relevant inner type in the `const`
        for ty in self.inner_emit_types() {
            ctx.record_ty(&ty);
        }

        // visit ant let bindings inside the `const`'s block
        for stmt in self.body_let_bind_stmts() {
            let attrs = stmt.autozng_attrs();
            attrs.visit(&stmt, ctx);
        }
    }
}

impl_attr_target!(crate::rustc_ext::HirConstItem, EmitItemsTarget);

impl EmitTarget for crate::rustc_ext::HirLetBindStmt {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        if !action.emit_visibility_na() {
            return false;
        }

        ctx.record_ty(&self.ty);

        true
    }

    fn name(&self) -> String {
        stable(self.hir_id.owner.to_def_id()).name()
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Emit)
    }
}

impl_attr_target!(crate::rustc_ext::HirLetBindStmt, EmitTarget);

// impl<'hir> EmitVisitable for rustc_hir::Stmt<'hir> {
//     fn emit(
//         &self,
//         _action: crate::attrs::emit::EmitAction,
//         _ctx: &crate::ctx::AutozngContext,
//     ) -> bool {
//         false
//     }
//     fn name(&self) -> String {
//         stable(self.hir_id.owner.to_def_id()).name()
//     }
//     fn implicit_action(&self) -> Option<crate::attrs::emit::EmitAction> {
//         Some(crate::attrs::emit::EmitAction::Emit)
//     }
// }

// impl_attributable!(rustc_hir::Stmt<'_>, EmitVisitable);

impl EmitTarget for crate::rustc_ext::HirTyAlias {
    fn emit(&self, _action: EmitAction, _ctx: &AutozngContext) -> bool {
        false
    }
    fn name(&self) -> String {
        stable(self.owner_id.to_def_id()).name()
    }
    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl_attr_target!(crate::rustc_ext::HirTyAlias, EmitTarget);
