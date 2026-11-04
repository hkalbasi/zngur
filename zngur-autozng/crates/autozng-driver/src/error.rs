use rustc_errors::{Diag, DiagMessage};
use rustc_middle::ty::TyCtxt;
use rustc_public::{rustc_internal::internal, ty::Span as RustcPublicSpan};
use rustc_span::Span;

use crate::driver::with_ctxt;

/// The internal [`Span`] of a `rustc_public` definition.
///
/// Used to record where an item was selected for emission (the item's own span), e.g. for the
/// implicit `Emit` attribute of an [`crate::attrs::attributes::emit::EmitTarget`].
pub fn span_of_def(def_id: rustc_public::DefId) -> Span {
    with_ctxt(|ctx| internal(ctx.tcx, def_id.span()))
}

/// Emit a warning attached to an internal [`Span`].
///
/// Used when the warning's primary span comes from an attribute (`#[autozng::emit]` /
/// `emit_items`) rather than from the offending item, which may have no `DefId` of its own.
pub fn emit_warning_for_span(span: Span, msg: impl Into<DiagMessage>) {
    with_ctxt(|ctx| {
        ctx.tcx.dcx().struct_span_warn(span, msg).emit();
    })
}

/// Emit an error attached to an internal [`Span`].
///
/// Used when a semantic error's primary span comes from an attribute (e.g. a
/// `#[autozng::layout(...)]` whose size does not match the type's real layout).
pub fn emit_error_for_span(span: Span, msg: impl Into<DiagMessage>) {
    with_ctxt(|ctx| {
        ctx.tcx.dcx().struct_span_err(span, msg).emit();
    })
}

/// Warn that a private item (or one of its functions) was selected for the interface.
///
/// `#[autozng::emit]`, `#[autozng::emit_on_implementers]` and `#[autozng::emit_items]` may name an
/// item that is not visible from the crate root (see [`crate::rustc_ext::Visibility`]). Such an
/// item is skipped (its path would not be visible to the generated interface), but not silently:
/// `span` is the attribute / item that selected it.
pub fn warn_private_item_skipped(span: Span) {
    emit_warning_for_span(
        span,
        "autozng: private traits and functions can be exposed in a zngur interface",
    );
}

#[allow(dead_code)] // counterpart to `with_struct_warning_for_rustc_public_span`
pub fn with_struct_error_for_rustc_public_span<R>(
    span: RustcPublicSpan,
    err: impl Into<DiagMessage>,
    f: impl for<'tcx> FnOnce(Diag<'tcx>, TyCtxt<'tcx>) -> R,
) -> R {
    with_ctxt(|ctx| {
        let span = internal(ctx.tcx, span);
        f(ctx.tcx.dcx().struct_span_err(span, err), ctx.tcx)
    })
}

pub fn with_struct_warning_for_rustc_public_span<R>(
    span: RustcPublicSpan,
    msg: impl Into<DiagMessage>,
    f: impl for<'tcx> FnOnce(Diag<'tcx, ()>, TyCtxt<'tcx>) -> R,
) -> R {
    with_ctxt(|ctx| {
        let span = internal(ctx.tcx, span);
        f(ctx.tcx.dcx().struct_span_warn(span, msg), ctx.tcx)
    })
}
