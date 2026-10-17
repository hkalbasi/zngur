//! `EmitTarget`/`RenameTarget`/`PolicyTarget` implementations for `rustc_public` types.
//!
//! ADTs, free functions (`FnDef`) and ADT inherent impl blocks (`ImplDef`) do real work:
//! emitting records the type / function / method into the [`AutozngContext`]. Methods are
//! recorded through [`rustc_public::ty::AssocItem`], which is visited while walking an impl
//! block's associated items. `TraitDef` emission is still a stub returning `false`.

use rustc_public::CrateDef;
use rustc_public::rustc_internal::{internal, stable};
use rustc_public::ty::AssocKind;

use crate::{
    attrs::{
        AttrCollectionExt, AutozngAttrsExt,
        attributes::{
            emit::{EmitAction, EmitTarget},
            policy::PolicyTarget,
            rename::RenameTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
    driver::with_ctxt,
    rustc_ext::{RustcPublicTyExt, VisibilityExt},
};

/// The simple (last path segment) name of a definition, used both for child `emit(name)`
/// matching and for `rename`.
fn simple_name(name: rustc_public::Symbol) -> String {
    name.rsplit("::").next().unwrap_or_default().to_owned()
}

impl PolicyTarget for rustc_public::Crate {}

impl_attr_target!(rustc_public::Crate, PolicyTarget);

impl EmitTarget for rustc_public::ty::AdtDef {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        let vis = self.visibility();
        if !action.emit_for_visibility(vis, ctx.config().default_emit_policy) {
            return false;
        }
        ctx.record_adt(self);
        true
    }

    fn name(&self) -> String {
        self.def_id().name()
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl RenameTarget for rustc_public::ty::AdtDef {
    fn item_name(&self) -> String {
        // `rename` only adjusts the item's own name (the last segment of its full path);
        // the crate/module path is preserved so the type stays discoverable.
        simple_name(CrateDef::name(self))
    }
}

impl_attr_target!(rustc_public::ty::AdtDef, EmitTarget, RenameTarget);

/// A free (non-associated) function in the crate, emitted as a `fn` entry in the `.zng` spec.
impl EmitTarget for rustc_public::ty::FnDef {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        // Associated functions are recorded through their impl block (`ImplDef`); only free
        // functions are recorded here.
        if self.associated_item().is_some() {
            return false;
        }
        let vis = self.visibility();
        if !action.emit_for_visibility(vis, ctx.config().default_emit_policy) {
            return false;
        }
        ctx.record_fn(*self);
        true
    }

    fn name(&self) -> String {
        simple_name(self.def_id().name())
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl RenameTarget for rustc_public::ty::FnDef {
    fn item_name(&self) -> String {
        simple_name(self.def_id().name())
    }
}

impl_attr_target!(rustc_public::ty::FnDef, EmitTarget, RenameTarget);

/// An ADT inherent impl block: emitting it walks its associated functions, each of which is
/// recorded (or skipped) according to its own attributes.
impl EmitTarget for rustc_public::ty::ImplDef {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        // An impl block has no visibility of its own; `skip` suppresses all its methods.
        if !action.emit_visibility_na() {
            return false;
        }
        for assoc_item in self.associated_items() {
            if !matches!(assoc_item.kind, AssocKind::Fn { .. }) {
                continue;
            }
            let attrs = assoc_item.autozng_attrs();
            attrs.visit(&assoc_item, ctx);
        }
        true
    }

    fn name(&self) -> String {
        self.def_id().name()
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl_attr_target!(rustc_public::ty::ImplDef, EmitTarget);

/// A method (associated function) of an impl block, recorded onto the type's [`TyRecord`].
///
/// [`TyRecord`]: crate::ctx::TyRecord
impl EmitTarget for rustc_public::ty::AssocItem {
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool {
        // Only methods are handled; associated constants/types are not emitted yet.
        if !matches!(self.kind, AssocKind::Fn { .. }) {
            return false;
        }
        let vis = self.visibility();
        if !action.emit_for_visibility(vis, ctx.config().default_emit_policy) {
            return false;
        }
        with_ctxt(|driver| {
            let tcx = driver.tcx;
            let own_def_id = internal(tcx, self.def_id.def_id());
            // Only inherent impl methods are recorded for now.
            let Some(impl_def_id) = tcx.impl_of_assoc(own_def_id) else {
                return false;
            };
            let impl_def = rustc_public::ty::ImplDef(stable(impl_def_id));
            let self_ty =
                stable(tcx.type_of(impl_def_id).instantiate_identity().skip_norm_wip());
            let fn_def = rustc_public::ty::FnDef(self.def_id.def_id());
            ctx.record_impl_method(self_ty.base_ty(), impl_def, fn_def, None);
            true
        })
    }

    fn name(&self) -> String {
        simple_name(self.def_id.def_id().name())
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl RenameTarget for rustc_public::ty::AssocItem {
    fn item_name(&self) -> String {
        simple_name(self.def_id.def_id().name())
    }
}

impl_attr_target!(rustc_public::ty::AssocItem, EmitTarget, RenameTarget);

// TODO: trait emission is unimplemented; always returns `false` (even for `emit`-tagged
// public traits, which per spec should be emitted).
impl EmitTarget for rustc_public::ty::TraitDef {
    fn emit(&self, _action: EmitAction, _ctx: &AutozngContext) -> bool {
        false
    }

    fn name(&self) -> String {
        self.def_id().name()
    }
}

impl_attr_target!(rustc_public::ty::TraitDef, EmitTarget);
