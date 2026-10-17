//! `EmitTarget`/`RenameTarget`/`PolicyTarget` implementations for `rustc_public` types.
//!
//! ADTs, free functions (`FnDef`) and ADT inherent impl blocks (`ImplDef`) do real work:
//! emitting records the type / function / method into the [`AutozngContext`]. Methods are
//! recorded through [`rustc_public::ty::AssocItem`], which is visited while walking an impl
//! block's associated items. `TraitDef` emission is still a stub returning `false`.

use rustc_public::CrateDef;
use rustc_public::rustc_internal::{internal, stable};
use rustc_public::ty::AssocKind;
use rustc_span::Span;

use crate::{
    attrs::{
        AttrCollectionExt, AutozngAttrsExt,
        attributes::{
            emit::{EmitAction, EmitOnImplTarget, EmitTarget},
            policy::PolicyTarget,
            rename::RenameTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
    driver::with_ctxt,
    error::{span_of_def, warn_private_item_skipped},
    rustc_ext::{FieldDefExt, ImplDefExt, RustcPublicTyExt, VisibilityExt},
};

/// The simple (last path segment) name of a definition, used both for child `emit(name)`
/// matching and for `rename`.
fn simple_name(name: rustc_public::Symbol) -> String {
    name.rsplit("::").next().unwrap_or_default().to_owned()
}

impl PolicyTarget for rustc_public::Crate {}

impl_attr_target!(rustc_public::Crate, PolicyTarget);

impl EmitTarget for rustc_public::ty::AdtDef {
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool {
        let vis = self.visibility();
        if !action.emit_for_visibility_or_warn(vis, ctx.config().default_emit_policy, span) {
            return false;
        }
        ctx.record_adt(self, self.emit_span());
        true
    }

    fn name(&self) -> String {
        self.def_id().name()
    }

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id())
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
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool {
        // Associated functions are recorded through their impl block (`ImplDef`); only free
        // functions are recorded here.
        if self.associated_item().is_some() {
            return false;
        }
        let vis = self.visibility();
        if !action.emit_for_visibility_or_warn(vis, ctx.config().default_emit_policy, span) {
            return false;
        }
        ctx.record_fn(*self, None, self.emit_span());
        true
    }

    fn name(&self) -> String {
        simple_name(self.def_id().name())
    }

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id())
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
    fn emit(&self, action: EmitAction, _span: Span, ctx: &AutozngContext) -> bool {
        // An impl block has no visibility of its own; `skip` suppresses all its methods.
        if !action.emit_visibility_na() {
            return false;
        }
        // Only an *inherent* impl records its methods directly (it is reached from `visit_adt`).
        // A trait impl's methods are reached through the trait (`emit` / `emit_items` /
        // `emit_on_implementers`); `visit_impl` walks trait impls only for the latter, so its
        // methods must not be recorded here as if they were inherent.
        if self.opt_trait_ref().is_some() {
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

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id())
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

/// `#[autozng::emit_on_implementers]` on a trait impl block: record the impl's clauses so the
/// trait's functions are emitted on every type the impl applies to.
impl EmitOnImplTarget for rustc_public::ty::ImplDef {
    fn emit_on_impl(&self, _span: Span, ctx: &AutozngContext) {
        let Some(trait_ref) = self.opt_trait_ref() else {
            return;
        };
        ctx.record_emit_for_impl(*self, trait_ref, self.emit_span());
    }
}

impl_attr_target!(rustc_public::ty::ImplDef, EmitTarget, EmitOnImplTarget);

/// A method (associated function) of an impl block, recorded onto the type's [`TyRecord`].
///
/// [`TyRecord`]: crate::ctx::TyRecord
impl EmitTarget for rustc_public::ty::AssocItem {
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool {
        // Only methods are handled; associated constants/types are not emitted yet.
        if !matches!(self.kind, AssocKind::Fn { .. }) {
            return false;
        }
        let vis = self.visibility();
        if !action.emit_for_visibility_or_warn(vis, ctx.config().default_emit_policy, span) {
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
            let self_ty = stable(
                tcx.type_of(impl_def_id)
                    .instantiate_identity()
                    .skip_norm_wip(),
            );
            let fn_def = rustc_public::ty::FnDef(self.def_id.def_id());
            ctx.record_impl_method(self_ty.base_ty(), impl_def, fn_def, None, self.emit_span());
            true
        })
    }

    fn name(&self) -> String {
        simple_name(self.def_id.def_id().name())
    }

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id.def_id())
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

/// A field of an ADT (struct/union field, or enum variant field), recorded onto the owning
/// type's [`TyRecord`].
///
/// [`TyRecord`]: crate::ctx::TyRecord
impl EmitTarget for rustc_public::ty::FieldDef {
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool {
        let vis = self.visibility();
        if !action.emit_for_visibility_or_warn(vis, ctx.config().default_emit_policy, span) {
            return false;
        }
        let Some(adt) = self.adt_def() else {
            return false;
        };
        ctx.record_field(adt.ty().base_ty(), self.clone(), self.emit_span());
        true
    }

    fn name(&self) -> String {
        self.name.to_string()
    }

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id())
    }

    fn implicit_action(&self) -> Option<EmitAction> {
        Some(EmitAction::Default)
    }
}

impl RenameTarget for rustc_public::ty::FieldDef {
    fn item_name(&self) -> String {
        self.name.to_string()
    }
}

impl_attr_target!(rustc_public::ty::FieldDef, EmitTarget, RenameTarget);

/// A trait declaration: recorded only when explicitly tagged `emit` (a trait's implicit action
/// is `None`, see [`EmitTarget::implicit_action`]) and only when it is `pub`/`pub(crate)`.
///
/// Recording the trait is what makes the visitor walk its associated functions (see
/// `CrateAnalysisVisitor::visit_trait`); the functions themselves are recorded with
/// [`AutozngContext::record_trait_method`].
impl EmitTarget for rustc_public::ty::TraitDef {
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool {
        let vis = self.visibility();
        if !action.emit_for_visibility_or_warn(vis, crate::config::DefaultEmitPolicy::None, span) {
            return false;
        }
        ctx.record_trait(*self, self.emit_span());
        true
    }

    fn name(&self) -> String {
        self.def_id().name()
    }

    fn emit_span(&self) -> Span {
        span_of_def(self.def_id())
    }

    /// A public trait is never emitted implicitly: unlike an ADT, it must carry an explicit
    /// `#[autozng::emit]`.
    fn implicit_action(&self) -> Option<EmitAction> {
        None
    }
}

/// `#[autozng::emit_on_implementers]` on a `pub`/`pub(crate)` trait declaration: record the trait
/// so its functions are emitted on every implementing type.
///
/// A private trait is not exportable (its path would not be visible to the generated interface),
/// so it is skipped — but not silently: the attribute is warned about.
impl EmitOnImplTarget for rustc_public::ty::TraitDef {
    fn emit_on_impl(&self, span: Span, ctx: &AutozngContext) {
        let vis = self.visibility();
        if !EmitAction::Emit.emit_for_visibility(vis, crate::config::DefaultEmitPolicy::None) {
            warn_private_item_skipped(span);
            return;
        }
        ctx.record_emit_for_trait(*self, self.emit_span());
    }
}

impl_attr_target!(rustc_public::ty::TraitDef, EmitTarget, EmitOnImplTarget);
