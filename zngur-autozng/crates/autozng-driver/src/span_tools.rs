use rustc_span::{BytePos, Pos, Span};

use crate::{driver::with_ctxt, error::AutozngError};

#[derive(Clone, Debug, Copy, PartialEq, Hash)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> From<rustc_span::Spanned<T>> for Spanned<T> {
    fn from(value: rustc_span::Spanned<T>) -> Self {
        Spanned {
            value: value.node,
            span: value.span,
        }
    }
}

impl<T> Spanned<T> {
    pub fn map<U, F>(self, f: F) -> Spanned<U>
    where
        F: FnOnce(T) -> U,
    {
        Spanned {
            value: f(self.value),
            span: self.span,
        }
    }
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            value: &self.value,
            span: self.span,
        }
    }
}

impl<T> Spanned<Result<T, AutozngError>> {
    /// transpose the result, leaving spans unchanged
    pub fn transpose(self) -> Result<Spanned<T>, AutozngError> {
        match self.value {
            Ok(value) => {
                let span = self.span;
                Ok(Spanned { value, span })
            }
            Err(err) => Err(err),
        }
    }
}

pub fn respan<T>(span: Span, value: T) -> Spanned<T> {
    Spanned { value, span }
}

pub trait StableRustcPublicSpanExt {
    fn stable(self) -> rustc_public::ty::Span;
}

impl StableRustcPublicSpanExt for Span {
    fn stable(self) -> rustc_public::ty::Span {
        rustc_public::rustc_internal::stable(self)
    }
}

pub trait OffsetRustcSpanExt {
    fn offset_lo(&self, offset: usize) -> Span;
    fn offset_hi(&self, offset: usize) -> Span;
    fn offset_by(&self, offset: usize) -> Span {
        self.offset_lo(offset).offset_hi(offset)
    }
    fn offset_inside(&self, outer: Span) -> Span;
}

impl OffsetRustcSpanExt for Span {
    fn offset_lo(&self, offset: usize) -> Span {
        let lo = BytePos::from_usize(self.lo().to_usize() + offset);
        self.with_lo(lo)
    }
    fn offset_hi(&self, offset: usize) -> Span {
        let hi = BytePos::from_usize(self.hi().to_usize() + offset);
        self.with_hi(hi)
    }
    fn offset_inside(&self, outer: Span) -> Span {
        let (lo_offset, hi_offset) = with_ctxt(|ctx| {
            let source_map = ctx.tcx.sess.source_map();
            (
                source_map.lookup_byte_offset(self.lo()).pos.to_usize(),
                source_map.lookup_byte_offset(self.hi()).pos.to_usize(),
            )
        });
        let lo = BytePos::from_usize(outer.lo().to_usize() + lo_offset);
        let hi = BytePos::from_usize(outer.lo().to_usize() + hi_offset);
        outer.with_lo(lo).with_hi(hi)
    }
}

pub trait RespanedInsideExt {
    fn respaned_inside(&self, outer: Span) -> Self;
}

impl RespanedInsideExt for rustc_ast::tokenstream::TokenStream {
    fn respaned_inside(&self, outer: Span) -> Self {
        self.iter()
            .map(|tree| tree.respaned_inside(outer))
            .collect()
    }
}

impl RespanedInsideExt for rustc_ast::tokenstream::TokenTree {
    fn respaned_inside(&self, outer: Span) -> Self {
        match self.clone() {
            rustc_ast::tokenstream::TokenTree::Token(token, spacing) => {
                rustc_ast::tokenstream::TokenTree::Token(token.respaned_inside(outer), spacing)
            }
            rustc_ast::tokenstream::TokenTree::Delimited(
                delim_span,
                delim_spacing,
                delim,
                stream,
            ) => rustc_ast::tokenstream::TokenTree::Delimited(
                delim_span.respaned_inside(outer),
                delim_spacing,
                delim,
                stream.respaned_inside(outer),
            ),
        }
    }
}

impl RespanedInsideExt for rustc_ast::token::Token {
    fn respaned_inside(&self, outer: Span) -> Self {
        rustc_ast::token::Token {
            kind: self.kind,
            span: self.span.offset_inside(outer),
        }
    }
}

impl RespanedInsideExt for rustc_ast::tokenstream::DelimSpan {
    fn respaned_inside(&self, outer: Span) -> Self {
        rustc_ast::tokenstream::DelimSpan {
            open: self.open.offset_inside(outer),
            close: self.close.offset_inside(outer),
        }
    }
}
