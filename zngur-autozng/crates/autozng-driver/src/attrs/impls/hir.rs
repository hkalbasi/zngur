//! `EmitTarget`/`EmitItemsTarget`/`RenameTarget` implementations for HIR-backed wrappers.
//!
//! `HirConstItem` (via `emit_items`) records enumerated inner types and walks `let`
//! bindings; `HirLetBindStmt` records its own type and is re-nameable (its `rename` is
//! collected from the statement's `HirId`). `HirTyAlias::emit` is a **stub returning
//! `false`** — `ty` aliases are visited but not emitted.

use rustc_public::CrateDef;
use rustc_public::rustc_internal::stable;
use rustc_public::ty::{RigidTy, TyKind};
use rustc_span::Span;

use rustc_middle::ty::TypeVisitableExt;

use crate::{
    attrs::{
        AttrCollectionExt, AutozngAttrsExt,
        attributes::{
            emit::{EmitAction, EmitTarget},
            emit_items::EmitItemsTarget,
            rename::RenameTarget,
            walk_filter::WalkFilterTarget,
        },
        macros::impl_attr_target,
    },
    ctx::AutozngContext,
    driver::with_ctxt,
    error::{emit_warning_for_span, warn_private_item_skipped},
    metadata::WalkFilterRecord,
    rustc_ext::{FnDefExt, RustcInternalEx, RustcPublicTyExt, Visibility, VisibilityExt},
    walk::{RequestedAs, VisitReason, WalkFilter},
};

impl EmitItemsTarget for crate::rustc_ext::HirConstItem {
    fn emit(&self, emit_traits: bool, ctx: &AutozngContext) {
        // The recorded span is the `const` item itself (the item tagged by `emit_items`): the
        // enumerated types have no source span of their own.
        let span = self.span();
        // A `walk_filter` on the const applies to every enumerated item. It is stored on the
        // `emit_items` item (attributes are visited before `emit_items`, see
        // `attrs::process_attrs`), and selected here so the item's children inherit it.
        let filter = ctx.walk_filter_for(self);
        let emit = |filter: Option<WalkFilter>| {
            // record any relevant inner type in the `const`
            for ty in self.inner_emit_types() {
                // With `emit_traits = true` the enumerated items are concrete trait
                // instantiations rather than types: a generic trait is only representable in
                // `.zng` once all of its type parameters are known, so every trait object in an
                // item names its principal trait. The `dyn` object itself is not then recorded as
                // a type.
                if emit_traits {
                    let traits = ty.trait_objects();
                    if !traits.is_empty() {
                        for trait_ref in traits {
                            record_trait_instantiation(trait_ref, span, ctx);
                        }
                        continue;
                    }
                }
                // A slice / `str` has no ADT to walk, so nothing else would record its methods.
                // Queue it explicitly; the walk then applies the default (or the user's) filter.
                // Queue before recording: recording marks the type as already seen, which would
                // otherwise drop the request.
                if ty.adt_def().is_none()
                    && matches!(
                        ty.base_ty().kind(),
                        TyKind::RigidTy(RigidTy::Slice(_) | RigidTy::Str)
                    )
                {
                    ctx.request_ty(ty, VisitReason::EmitItems, RequestedAs::Value, span);
                }
                ctx.record_ty_with_filter(&ty, span, true, filter.clone());
            }

            // visit any let bindings inside the `const`'s block
            for stmt in self.body_let_bind_stmts(emit_traits, filter.clone()) {
                // A `let` binding has no `DefId` of its own, so its attribute key resolves to the
                // enclosing `const`: `autozng_attrs` therefore returns the *const's* attributes
                // (forwarding any child-targeted ones onto the binding). The binding's own
                // implicit `Emit`/`Skip` action is applied separately, since the `DefId`-keyed
                // cache never computes it for a `let`.
                let attrs = stmt.autozng_attrs();
                attrs.visit(&stmt, ctx);
                if let Some(implicit) = stmt.implicit_emit_attr() {
                    implicit.visit(&stmt, ctx);
                }
            }
        };
        if let Some(filter) = filter {
            ctx.with_filter(self.ty, filter.clone(), span, || emit(Some(filter)));
        } else {
            emit(None);
        }
    }
}

impl WalkFilterTarget for crate::rustc_ext::HirConstItem {
    fn apply_walk_filter(&self, filter: WalkFilter, span: Span, ctx: &AutozngContext) {
        ctx.with_metadata_mut(self, |mut metadata| {
            metadata.walk_filter = Some(WalkFilterRecord { filter, span });
        });
    }
}

impl_attr_target!(
    crate::rustc_ext::HirConstItem,
    EmitItemsTarget,
    WalkFilterTarget
);

/// Record one trait object's principal trait as a concrete export target.
///
/// A trait is only exportable when it is `pub`/`pub(crate)`, mirroring `#[autozng::emit]`. A
/// private trait is skipped, but not silently: the item (or `let` binding) that selected it is
/// warned about.
fn record_trait_instantiation(
    trait_ref: rustc_public::ty::TraitRef,
    span: Span,
    ctx: &AutozngContext,
) {
    // A trait object behind a higher-ranked binder (`fn(&dyn Trait)`) keeps the binder's region
    // inside the object, and `rustc_public` cannot stabilize an escaping bound region. Ignore it
    // (with a warning) rather than aborting the analysis.
    if with_ctxt(|ctx| {
        trait_ref
            .clone()
            .internal(ctx.tcx)
            .has_escaping_bound_vars()
    }) {
        emit_warning_for_span(
            span,
            "autozng: a trait object behind a higher-ranked binder is not supported; name it directly with `&dyn Trait`",
        );
        return;
    }
    let vis = trait_ref.def_id.visibility();
    if EmitAction::Emit.emit_for_visibility(vis, crate::config::DefaultEmitPolicy::None) {
        ctx.record_concrete_trait(trait_ref, span);
    } else {
        warn_private_item_skipped(span);
    }
}

impl EmitTarget for crate::rustc_ext::HirLetBindStmt {
    fn emit(&self, action: EmitAction, _span: Span, ctx: &AutozngContext) -> bool {
        if !action.emit_visibility_na() {
            return false;
        }

        // The span of the `let` binding that selected the item.
        let span = self.emit_span();

        // With `emit_traits = true` a trait object in the binding's type names a concrete trait
        // instantiation instead of a type (see [`EmitItemsTarget`]). The binding is consumed even
        // when every named trait is private, so the `dyn` object never falls through to
        // `record_ty`.
        if self.emit_traits {
            let traits = self.ty.trait_objects();
            if !traits.is_empty() {
                for trait_ref in traits {
                    record_trait_instantiation(trait_ref, span, ctx);
                }
                return true;
            }
        }

        // A `let` binding inside an `emit_items` const enumerates either a concrete type, or a
        // monomorphization of a function (an fn item such as `foo::<u8>` / `Self::bar::<u8>`).
        let rustc_public::ty::TyKind::RigidTy(rustc_public::ty::RigidTy::FnDef(fn_def, generics)) =
            self.ty.kind()
        else {
            // A slice / `str` has no ADT to walk, so queue its interface explicitly (see
            // `EmitItemsTarget::emit`). Queue before recording: recording marks the type as
            // already seen, which would otherwise drop the request.
            if self.ty.adt_def().is_none()
                && matches!(
                    self.ty.base_ty().kind(),
                    TyKind::RigidTy(RigidTy::Slice(_) | RigidTy::Str)
                )
            {
                ctx.request_ty(self.ty, VisitReason::EmitItems, RequestedAs::Value, span);
            }
            ctx.record_ty_with_filter(&self.ty, span, true, self.walk_filter.clone());
            return true;
        };

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
            // A method of a private trait is not exportable (the trait's path would not be
            // visible to the generated interface), so it is skipped — but not silently: the
            // `let` binding that named it is warned about.
            if matches!(tref.def_id.visibility(), Visibility::PrivateAPI) {
                warn_private_item_skipped(span);
                return false;
            }
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

// TODO: `ty` alias emission is unimplemented; always returns `false`.
impl EmitTarget for crate::rustc_ext::HirTyAlias {
    fn emit(&self, _action: EmitAction, _span: Span, _ctx: &AutozngContext) -> bool {
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

impl_attr_target!(crate::rustc_ext::HirTyAlias, EmitTarget, RenameTarget);

impl RenameTarget for crate::rustc_ext::HirTyAlias {
    fn item_name(&self) -> String {
        self.ident.to_string()
    }

    fn method_carrier(&self) -> Option<rustc_public::ty::Ty> {
        Some(self.ty)
    }

    fn is_type_alias(&self) -> bool {
        true
    }
}
