use std::fmt::Display;

use itertools::{Either, Itertools};
use rustc_errors::{Diag, DiagMessage};
use rustc_middle::ty::TyCtxt;
use rustc_public::{rustc_internal::internal, ty::Span as RustcPublicSpan};
use rustc_span::{DUMMY_SP, ErrorGuaranteed, Span};

use crate::{attrs::AttrPath, driver::with_ctxt, span_tools::Spanned};

// The error type and its helpers are the framework for reporting attribute/analysis errors; the
// remaining callers are migrated as the passes grow.
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum AutozngError {
    #[error("Attribute `{attr_path}` not allowed on this item")]
    InvalidAttrContext { attr_path: AttrPath, span: Span },
    #[error("`{attr_path}` not a known autozng attribute")]
    UnknownAttr { attr_path: String, span: Span },
    #[error("failed to parse attribute arguments as meta items")]
    FailedToParseMeta(Span),
    #[error(r#"expected a `"str"` literal"#)]
    ExpectedLiteralStr(Span),
    #[error("{msg}")]
    Custom { msg: String, span: Span },
    #[error("error already reported")]
    AlreadyReported,
}

#[allow(dead_code)]
impl AutozngError {
    pub fn custom_w_dummy_span(msg: impl Display) -> Self {
        Self::Custom {
            msg: msg.to_string(),
            span: DUMMY_SP,
        }
    }

    pub fn with_span(self, span: Span) -> Self {
        match self {
            Self::InvalidAttrContext { attr_path, .. } => {
                Self::InvalidAttrContext { attr_path, span }
            }
            Self::UnknownAttr { attr_path, .. } => Self::UnknownAttr { attr_path, span },
            Self::FailedToParseMeta(_) => Self::FailedToParseMeta(span),
            Self::ExpectedLiteralStr(_) => Self::ExpectedLiteralStr(span),
            Self::Custom { msg, .. } => Self::Custom { msg, span },
            Self::AlreadyReported => self,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::InvalidAttrContext { span, .. } => *span,
            Self::UnknownAttr { span, .. } => *span,
            Self::FailedToParseMeta(span) => *span,
            Self::ExpectedLiteralStr(span) => *span,
            Self::Custom { span, .. } => *span,
            Self::AlreadyReported => DUMMY_SP,
        }
    }
}

impl From<Spanned<String>> for AutozngError {
    fn from(value: Spanned<String>) -> Self {
        AutozngError::Custom {
            msg: value.value,
            span: value.span,
        }
    }
}

impl From<ErrorGuaranteed> for AutozngError {
    fn from(_value: ErrorGuaranteed) -> Self {
        Self::AlreadyReported
    }
}

impl<'a> From<rustc_errors::Diag<'a>> for AutozngError {
    fn from(value: rustc_errors::Diag<'a>) -> Self {
        value.emit().into()
    }
}

#[allow(dead_code)] // used by the analysis passes as they migrate onto `AutozngError`
pub fn pull_and_emit_errors_from_results<T>(
    results: impl IntoIterator<Item = Result<T, AutozngError>>,
) -> Vec<T> {
    with_ctxt(|ctx| {
        let dcx = ctx.tcx.dcx();
        let (errs, results): (Vec<AutozngError>, Vec<T>) =
            results.into_iter().partition_map(|result| match result {
                Err(err) => Either::Left(err),
                Ok(result) => Either::Right(result),
            });
        for err in errs {
            if !matches!(err, AutozngError::AlreadyReported) {
                eprintln!("Error for span: {:?}", err.span());
                let err = dcx.struct_span_err(err.span(), err.to_string());
                err.emit();
            }
        }
        results
    })
}

#[allow(dead_code)] // counterpart to `with_struct_warning_for_rustc_public_span`
pub fn emit_error_for_rustc_public_span(span: RustcPublicSpan, err: impl Into<DiagMessage>) {
    with_ctxt(|ctx| {
        let span = internal(ctx.tcx, span);
        ctx.tcx.dcx().struct_span_err(span, err).emit();
    })
}

/// Emit a warning attached to a `rustc_public` span (converted to an internal [`Span`]).
#[allow(dead_code)] // render currently warns through `emit_warning_for_span`
pub fn emit_warning_for_rustc_public_span(span: RustcPublicSpan, msg: impl Into<DiagMessage>) {
    with_ctxt(|ctx| {
        let span = internal(ctx.tcx, span);
        ctx.tcx.dcx().struct_span_warn(span, msg).emit();
    })
}

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
