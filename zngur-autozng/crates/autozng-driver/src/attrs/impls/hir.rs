//! `EmitTarget`/`EmitItemsTarget`/`RenameTarget` implementations for HIR-backed wrappers.
//!
//! `HirConstItem` (via `emit_items`) records enumerated inner types and walks `let`
//! bindings; `HirLetBindStmt` records its own type and is re-nameable (its `rename` is
//! collected from the statement's `HirId`). `HirTyAlias::emit` is a **stub returning
//! `false`** — `ty` aliases are visited but not emitted.

use rustc_public::CrateDef;
use rustc_public::rustc_internal::stable;
use rustc_span::Span;

use crate::{
    attrs::{
        AttrCollectionExt, AutozngAttrsExt,
        attributes::{
            emit::{EmitAction, EmitTarget},
            emit_items::EmitItemsTarget,
            rename::RenameTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
    driver::with_ctxt,
    rustc_ext::FnDefExt,
};

impl EmitItemsTarget for crate::rustc_ext::HirConstItem {
    fn emit(&self, ctx: &AutozngContext) {
        // The recorded span is the `const` item itself (the item tagged by `emit_items`): the
        // enumerated types have no source span of their own.
        let span = self.span();
        // record any relevant inner type in the `const`
        for ty in self.inner_emit_types() {
            ctx.record_ty(&ty, span);
        }

        // visit any let bindings inside the `const`'s block
        for stmt in self.body_let_bind_stmts() {
            // A `let` binding has no `DefId` of its own, so its attribute key resolves to the
            // enclosing `const`: `autozng_attrs` therefore returns the *const's* attributes
            // (forwarding any child-targeted ones onto the binding). The binding's own implicit
            // `Emit`/`Skip` action is applied separately, since the `DefId`-keyed cache never
            // computes it for a `let`.
            let attrs = stmt.autozng_attrs();
            attrs.visit(&stmt, ctx);
            if let Some(implicit) = stmt.implicit_emit_attr() {
                implicit.visit(&stmt, ctx);
            }
        }
    }
}

impl_attr_target!(crate::rustc_ext::HirConstItem, EmitItemsTarget);

impl EmitTarget for crate::rustc_ext::HirLetBindStmt {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        if !action.emit_visibility_na() {
            return false;
        }

        // A `let` binding inside an `emit_items` const enumerates either a concrete type, or a
        // monomorphization of a function (an fn item such as `foo::<u8>` / `Self::bar::<u8>`).
        let rustc_public::ty::TyKind::RigidTy(rustc_public::ty::RigidTy::FnDef(fn_def, generics)) =
            self.ty.kind()
        else {
            ctx.record_ty(&self.ty, self.emit_span());
            return true;
        };

        // The span of the `let` binding that selected the item.
        let span = self.emit_span();
        // `tref` is `Some` when the fn item is a trait method; `partial_args` are the fn item's
        // own generic args, present when it was monomorphized (`foo::<u8>`).
        let tref = fn_def.trait_ref(&generics);
        let partial_args = (!generics.0.is_empty()).then(|| generics.clone());

        // Inside an impl const the fn item is a method of *that* impl. Otherwise it is attached
        // to its own impl (an associated fn), to its own trait (a trait method), or recorded as
        // a free function.
        if let Some((self_ty, impl_def)) = self.impl_parent() {
            ctx.record_impl_method(self_ty, impl_def, fn_def, partial_args.clone(), span);
        } else if let Some((self_ty, impl_def)) = fn_def.impl_of() {
            ctx.record_impl_method(self_ty, impl_def, fn_def, partial_args.clone(), span);
        } else if let Some(tref) = tref {
            ctx.record_trait_type_fn(fn_def, tref, generics, span);
        } else {
            ctx.record_fn(fn_def, partial_args, span);
        }

        true
    }

    fn name(&self) -> String {
        stable(self.hir_id.owner.to_def_id()).name()
    }

    fn emit_span(&self) -> Span {
        with_ctxt(|ctx| ctx.tcx.hir_span(self.hir_id))
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Emit)
    }
}

/// A `let` binding inside an `#[autozng::emit_items]` const is re-nameable: the `rename` reaches
/// the function item's monomorphization (see [`crate::rustc_ext::HirLetBindStmt`]'s
/// `IntoMetadataKey`, which keys the export name by the fn's `DefId` + enumerated args).
impl RenameTarget for crate::rustc_ext::HirLetBindStmt {
    fn item_name(&self) -> String {
        // A plain `rename("x")` ignores the name; `rename(case = ...)` transforms the named
        // function's simple name, mirroring a `rename` on the method itself.
        if let rustc_public::ty::TyKind::RigidTy(rustc_public::ty::RigidTy::FnDef(fn_def, _)) =
            self.ty.kind()
        {
            return CrateDef::name(&fn_def)
                .rsplit("::")
                .next()
                .unwrap_or_default()
                .to_owned();
        }
        String::new()
    }
}

impl_attr_target!(crate::rustc_ext::HirLetBindStmt, EmitTarget, RenameTarget);

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

// TODO: `ty` alias emission is unimplemented; always returns `false`.
impl EmitTarget for crate::rustc_ext::HirTyAlias {
    fn emit(&self, _action: EmitAction, _ctx: &AutozngContext) -> bool {
        false
    }
    fn name(&self) -> String {
        stable(self.owner_id.to_def_id()).name()
    }
    fn emit_span(&self) -> Span {
        with_ctxt(|ctx| ctx.tcx.def_span(self.owner_id.def_id))
    }
    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl_attr_target!(crate::rustc_ext::HirTyAlias, EmitTarget);
