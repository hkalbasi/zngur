use rustc_ast::{DelimArgs, MetaItemLit};

use rustc_attr_ir::AttrItem;
use rustc_middle::ty::TyCtxt;
use rustc_public::{rustc_internal::stable, ty::LineInfo};
use rustc_session::parse::ParseSess;
use rustc_span::{ErrorGuaranteed, Span};

use crate::{
    attrs::{AUTOZNG_NAMESPACE, AttrBuilder, AttrPath, PATH_SEP},
    driver::with_ctxt,
    span_tools::{OffsetRustcSpanExt, RespanedInsideExt, Spanned, respan},
};

pub mod context;
pub mod diagnostics;
pub mod parser;
pub mod storage;

mod from_rustc {

    use super::*;
    use rustc_macros::Diagnostic;

    #[derive(Diagnostic)]
    #[diag("suffixed literals are not allowed in attributes")]
    #[help(
        "instead of using a suffixed literal (`1u8`, `1.0f32`, etc.), use an unsuffixed version (`1`, `1.0`, etc.)"
    )]
    pub(crate) struct SuffixedLiteralInAttribute {
        #[primary_span]
        pub span: Span,
    }

    pub fn expr_to_lit<'sess>(
        psess: &'sess ParseSess,
        expr: &rustc_ast::Expr,
        span: Span,
    ) -> Result<MetaItemLit, ErrorGuaranteed> {
        if let rustc_ast::ExprKind::Lit(token_lit) = expr.kind {
            let res = MetaItemLit::from_token_lit(token_lit, expr.span);
            match res {
                Ok(lit) => {
                    if token_lit.suffix.is_some() {
                        Err(psess
                            .dcx()
                            .create_err(SuffixedLiteralInAttribute { span: lit.span })
                            .emit())
                    } else if lit.kind.is_unsuffixed() {
                        Ok(lit)
                    } else {
                        Err(psess
                            .dcx()
                            .create_err(SuffixedLiteralInAttribute { span: lit.span })
                            .emit())
                    }
                }
                Err(err) => {
                    let err = rustc_session::diagnostics::create_lit_error(
                        psess, err, token_lit, expr.span,
                    );
                    Err(err.emit())
                }
            }
        } else {
            // Example cases:
            // - `#[foo = 1+1]`: results in `ast::ExprKind::BinOp`.
            // - `#[foo = include_str!("nonexistent-file.rs")]`:
            //   results in `ast::ExprKind::Err`.
            let msg = "attribute value must be a literal";
            let mut err = psess.dcx().struct_span_err(span, msg);

            // Suggest adding quotation marks to turn an identifier into a string literal
            if let rustc_ast::ExprKind::Path(None, ref path) = expr.kind
                && let [_] = path.segments.as_slice()
            {
                err.multipart_suggestion(
                    "you might have meant to write a string literal",
                    vec![
                        (expr.span.shrink_to_lo(), "\"".to_string()),
                        (expr.span.shrink_to_hi(), "\"".to_string()),
                    ],
                    rustc_lint::Applicability::MaybeIncorrect,
                );
            }

            Err(err.emit())
        }
    }
}

#[derive(Clone)]
pub struct PartialNameValue {
    pub eq_span: Span,
    pub span: Span,
    pub lit: MetaItemLit,
}

/// a partially parsed Attribute for autozng.
#[derive(Clone)]
pub enum PartialParsedAttrKind {
    Path,
    List { args: DelimArgs },
    NameValue { args: PartialNameValue },
}

#[derive(Clone)]
pub struct PartialParsedAttr {
    pub path: AttrPath,
    pub span: Span,
    pub kind: PartialParsedAttrKind,
}

impl std::fmt::Debug for PartialParsedAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)?;
        match &self.kind {
            PartialParsedAttrKind::List { args } => {
                let args = rustc_ast_pretty::pprust::tts_to_string(&args.tokens);
                write!(f, "({args:?})")
            }
            PartialParsedAttrKind::NameValue { args: value, .. } => {
                write!(f, " = {}", value.lit.as_token_lit().to_string())
            }
            _ => Ok(()),
        }
    }
}

pub fn parse_attr_item_ir(attr: &rustc_attr_ir::AttrItem) -> PartialParsedAttr {
    let span = attr.span;
    let path = AttrPath::from(&attr.path.segments);
    match &attr.args {
        rustc_attr_ir::AttrArgs::Empty => PartialParsedAttr {
            path,
            span,
            kind: PartialParsedAttrKind::Path,
        },
        rustc_attr_ir::AttrArgs::Delimited(args) => {
            // get enclosing span of tokens between delimiters

            PartialParsedAttr {
                path,
                span,
                kind: PartialParsedAttrKind::List { args: args.clone() },
            }
        }
        rustc_attr_ir::AttrArgs::Eq { eq_span, expr, .. } => PartialParsedAttr {
            path,
            span,
            kind: PartialParsedAttrKind::NameValue {
                args: PartialNameValue {
                    eq_span: *eq_span,
                    span: expr.span,
                    lit: expr.clone(),
                },
            },
        },
    }
}

/// parse the attribute from a doc comment. there should only ever be one in the stream.
/// Because rustc only gives us one at a time and we filter out doc lines that could have more than one.
/// return a result with error message and range inside attr source
pub fn parse_attr_str(source: &str, span: Span) -> Result<PartialParsedAttr, ErrorGuaranteed> {
    let stable_span = stable(span);
    let source_file_name = stable_span.get_filename();
    let LineInfo {
        start_line,
        start_col,
        end_line,
        end_col,
    } = stable_span.get_lines();

    let stream_filename =
        format!("{source_file_name}:{start_line}:{start_col}-{end_line}:{end_col}");

    let attr = with_ctxt(|ctx| -> Result<rustc_ast::Attribute, ErrorGuaranteed> {
        let psess = &ctx.tcx.sess.psess;

        let stream = match rustc_parse::source_str_to_stream(
            psess,
            rustc_span::FileName::Custom(stream_filename),
            source.to_string(),
            None,
        ) {
            Ok(stream) => stream.respaned_inside(span),
            Err(diags) => {
                for diag in diags {
                    let _ = diag.emit();
                }
                return Err(psess
                    .dcx()
                    .struct_span_err(span, "failed to parse doc comment attr to tokens")
                    .emit());
            }
        };

        let mut parser = rustc_parse::parser::Parser::new(psess, stream, None);

        match parser.parse_attribute(rustc_parse::parser::attr::InnerAttrPolicy::Forbidden(None)) {
            Ok(attr) => Ok(attr),
            Err(err) => Err(err.emit()),
        }
    })?;

    parse_attr_item_ast(attr.get_normal_item())
}

pub fn parse_attr_item_ast(
    attr: &rustc_ast::AttrItem,
) -> Result<PartialParsedAttr, ErrorGuaranteed> {
    let span = attr.span;
    let path = AttrPath::from(attr.path.segments.iter().map(|seg| &seg.ident.name));
    let partial = match &attr.args {
        rustc_ast::AttrArgs::Empty => PartialParsedAttr {
            path,
            span,
            kind: PartialParsedAttrKind::Path,
        },
        rustc_ast::AttrArgs::Delimited(delim) => {
            // get enclosing span of tokens between delimiters
            let span = delim.dspan.entire();

            PartialParsedAttr {
                path,
                span,
                kind: PartialParsedAttrKind::List {
                    args: delim.clone(),
                },
            }
        }
        rustc_ast::AttrArgs::Eq { eq_span, expr, .. } => PartialParsedAttr {
            path,
            span,
            kind: PartialParsedAttrKind::NameValue {
                args: PartialNameValue {
                    eq_span: *eq_span,
                    span: expr.span,
                    lit: with_ctxt(|ctx| {
                        let psess = &ctx.tcx.sess.psess;
                        from_rustc::expr_to_lit(psess, expr, expr.span)
                    })?,
                },
            },
        },
    };
    Ok(partial)
}
