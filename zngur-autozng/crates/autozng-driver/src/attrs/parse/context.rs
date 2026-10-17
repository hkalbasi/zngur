//! adapted out of rustc_attr_parsing as the interface is private

use rustc_ast::MetaItemLit;
use rustc_attr_parsing::AttributeTemplate;
use rustc_errors::{DiagCtxtHandle, Diagnostic};
use rustc_session::Session;
use rustc_span::{ErrorGuaranteed, Span};

use crate::{
    attrs::{
        AttrPath,
        parse::{
            diagnostics::{AttributeParseError, AttributeParseErrorReason},
            parser::{
                ArgParser, ExpectBoolLiteral, ExpectNameValue, ExpectStringLiteral,
                MetaItemListParser, MetaItemOrLitParser,
            },
        },
    },
    span_tools::Spanned,
};

pub struct AcceptContext<'f, 'sess> {
    pub sess: &'sess Session,
    pub dcx: DiagCtxtHandle<'sess>,
    pub template: &'f AttributeTemplate,
    pub attr_path: &'f AttrPath,
    pub span: Span,
}

// The `expect_*` helpers mirror the (private) `rustc_attr_parsing` context; a few are not used
// by autozng's own attribute parsers yet.
#[allow(dead_code)]
impl<'f, 'sess> AcceptContext<'f, 'sess> {
    pub fn expect_single_element_list<'arg>(
        &mut self,
        arg: &'arg ArgParser,
        span: Span,
    ) -> Option<&'arg MetaItemOrLitParser> {
        let ArgParser::List(l) = arg else {
            self.expected_list(span, arg);
            return None;
        };

        let Some(single) = l.as_single() else {
            self.expected_single_argument(span, l.len());
            return None;
        };
        Some(single)
    }

    pub fn expect_list<'arg>(
        &mut self,
        args: &'arg ArgParser,
        span: Span,
    ) -> Option<&'arg MetaItemListParser> {
        let list = args.as_list();
        if list.is_none() {
            self.expected_list(span, args);
        }
        list
    }

    pub fn expect_single<'arg>(
        &mut self,
        list: &'arg MetaItemListParser,
    ) -> Option<&'arg MetaItemOrLitParser> {
        let single = list.as_single();
        if single.is_none() {
            self.expected_single_argument(list.span, list.len());
        }
        single
    }

    pub fn expect_name_value<'arg, Arg>(
        &mut self,
        arg: &'arg Arg,
        span: Span,
        name: Option<&'arg str>,
    ) -> Option<Arg::Output<'arg>>
    where
        Arg: ExpectNameValue,
    {
        arg.expect_name_value(self, span, name)
    }

    pub fn expect_no_args(&mut self, arg: &ArgParser) -> Option<()> {
        if let Err(span) = arg.as_no_args() {
            self.expected_no_args(span);
            return None;
        }

        Some(())
    }

    pub fn expect_string_literal<Arg>(&mut self, arg: &Arg) -> Option<Spanned<String>>
    where
        Arg: ExpectStringLiteral,
    {
        arg.expect_string_literal(self)
    }

    pub fn expect_bool_literal<Arg>(&mut self, arg: &Arg) -> Option<Spanned<bool>>
    where
        Arg: ExpectBoolLiteral,
    {
        arg.expect_bool_literal(self)
    }
}

// The `expected_*` error constructors mirror `rustc_attr_parsing`; the unused ones are kept so
// the framework stays complete.
#[allow(dead_code)]
impl<'f, 'sess> AcceptContext<'f, 'sess> {
    pub fn expected_list(&mut self, span: Span, args: &ArgParser) -> ErrorGuaranteed {
        let span = match args {
            ArgParser::NoArgs => span,
            ArgParser::List(list) => list.span,
            ArgParser::NameValue(nv) => nv.args_span(),
        };
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedList)
    }

    pub fn expected_list_with_num_args_or_more(
        &mut self,
        args: usize,
        span: Span,
    ) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedListWithNumArgsOrMore { args },
        )
    }

    pub fn expected_list_or_no_args(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedListOrNoArgs)
    }

    pub fn expected_nv_or_no_args(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedNameValueOrNoArgs)
    }

    pub fn expected_non_empty_string_literal(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedNonEmptyStringLiteral,
        )
    }

    pub fn expected_no_args(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedNoArgs)
    }

    pub fn expected_identifier(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedIdentifier)
    }

    pub fn expected_name_value(&mut self, span: Span, name: Option<&str>) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedNameValue(name))
    }

    /// Emit an error that a `name = value` argument is missing in a list of name-value pairs.
    pub fn missing_name_value(&mut self, span: Span, name: &str) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::MissingNameValue(name))
    }

    /// Emit an error that a `name = value` pair was found where that name was already seen.
    pub fn duplicate_key(&mut self, span: Span, key: &str) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::DuplicateKey(key))
    }

    /// An error that should be emitted when a [`MetaItemOrLitParser`]
    /// was expected *not* to be a literal, but instead a meta item.
    pub fn expected_not_literal(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedNotLiteral)
    }

    pub fn expected_single_argument(
        &mut self,
        span: Span,
        provided_arguments: usize,
    ) -> ErrorGuaranteed {
        let reason = if provided_arguments == 0 {
            AttributeParseErrorReason::ExpectedArgument
        } else {
            AttributeParseErrorReason::ExpectedSingleArgument
        };

        self.emit_parse_error(span, reason)
    }

    pub fn expected_at_least_one_argument(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedAtLeastOneArgument)
    }

    /// Produces an error along the lines of `expected one of [foo, meow]`
    pub fn expected_specific_argument(
        &mut self,
        span: Span,
        possibilities: &[&str],
    ) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedSpecificArgument {
                possibilities,
                strings: false,
                list: false,
            },
        )
    }

    /// Produces an error along the lines of `expected one of [foo, meow] as an argument`.
    /// i.e. slightly different wording to [`expected_specific_argument`](Self::expected_specific_argument).
    pub(crate) fn expected_specific_argument_and_list(
        &mut self,
        span: Span,
        possibilities: &[&str],
    ) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedSpecificArgument {
                possibilities,
                strings: false,
                list: true,
            },
        )
    }

    /// produces an error along the lines of `expected one of ["foo", "meow"]`
    pub(crate) fn expected_specific_argument_strings(
        &mut self,
        span: Span,
        possibilities: &[&str],
    ) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedSpecificArgument {
                possibilities,
                strings: true,
                list: false,
            },
        )
    }

    pub fn expected_string_literal(
        &mut self,
        span: Span,
        actual_literal: Option<&MetaItemLit>,
    ) -> ErrorGuaranteed {
        self.emit_parse_error(
            span,
            AttributeParseErrorReason::ExpectedStringLiteral {
                byte_string: actual_literal.and_then(|i| {
                    i.kind
                        .is_bytestr()
                        .then(|| self.sess.source_map().start_point(i.span))
                }),
            },
        )
    }

    pub(crate) fn expected_integer_literal(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedIntegerLiteral)
    }

    pub fn expected_bool_literal(&mut self, span: Span) -> ErrorGuaranteed {
        self.emit_parse_error(span, AttributeParseErrorReason::ExpectedBoolLiteral)
    }
}

impl<'f, 'sess> AcceptContext<'f, 'sess> {
    fn suggestions(&self) -> Vec<String> {
        self.template.suggestions(
            rustc_attr_parsing::ParsedDescription::Attribute,
            rustc_ast::Safety::Safe(self.span),
            self.attr_path,
        )
    }

    fn emit_parse_error(
        &mut self,
        mut span: Span,
        reason: AttributeParseErrorReason,
    ) -> ErrorGuaranteed {
        let suggestions = self.suggestions();

        // If the span is the full attribute (including the `#[`/`]` delimiters) shrink it to
        // exclude those delimiters, because that's what we want in error messages.
        if !self.span.contains(span) {
            span = self.span;
        }

        self.emit_err(AttributeParseError {
            span,
            template: *self.template,
            path: self.attr_path.clone(),
            reason,
            suggestions,
        })
    }

    fn emit_err(&self, diag: impl Diagnostic<'sess>) -> ErrorGuaranteed {
        self.dcx.create_err(diag).emit()
    }
}
