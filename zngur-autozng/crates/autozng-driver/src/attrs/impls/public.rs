//! `EmitTarget`/`RenameTarget`/`PolicyTarget` implementations for `rustc_public` types.
//!
//! Only ADTs do real work (`EmitTarget::emit` records the type). `TraitDef` and `AssocItem`
//! `emit` are **stubs returning `false`**, which is why trait-related and method/field
//! emission never reach the context yet.

use rustc_public::CrateDef;

use crate::{
    attrs::{
        attributes::{
            emit::{EmitAction, EmitTarget},
            policy::PolicyTarget,
            rename::RenameTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
    rustc_ext::VisibilityExt,
};

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
        CrateDef::name(self)
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_owned()
    }
}

impl_attr_target!(rustc_public::ty::AdtDef, EmitTarget, RenameTarget);

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

// TODO: associated-item emission is unimplemented; always returns `false`, so no methods
// or fields are ever recorded.
impl EmitTarget for rustc_public::ty::AssocItem {
    fn emit(&self, _action: EmitAction, _ctx: &AutozngContext) -> bool {
        false
    }

    fn name(&self) -> String {
        self.def_id.def_id().name()
    }
}

impl_attr_target!(rustc_public::ty::AssocItem, EmitTarget);
