//! Mostly copied out of rustc_attr_parsing as the interface is private

use rustc_attr_parsing::AttributeTemplate;
use rustc_errors::{Diag, Diagnostic};
use rustc_lint::Applicability;
use rustc_span::Span;

use crate::attrs::AttrPath;

// Mirrors `rustc_attr_parsing`'s reason list; not every reason is produced by autozng yet.
#[allow(dead_code)]
pub enum AttributeParseErrorReason<'a> {
    ExpectedNoArgs,
    ExpectedStringLiteral {
        byte_string: Option<Span>,
    },
    ExpectedIntegerLiteral,
    ExpectedIntegerLiteralInRange {
        lower_bound: isize,
        upper_bound: isize,
    },
    ExpectedAtLeastOneArgument,
    ExpectedArgument,
    ExpectedSingleArgument,
    ExpectedList,
    ExpectedListOrNoArgs,
    ExpectedListWithNumArgsOrMore {
        args: usize,
    },
    ExpectedNameValueOrNoArgs,
    ExpectedNonEmptyStringLiteral,
    ExpectedNotLiteral,
    ExpectedNameValue(Option<&'a str>),
    MissingNameValue(&'a str),
    DuplicateKey(&'a str),
    ExpectedSpecificArgument {
        possibilities: &'a [&'a str],
        strings: bool,
        /// Should we tell the user to write a list when they didn't?
        list: bool,
    },
    ExpectedIdentifier,
}

pub struct AttributeParseError<'a> {
    pub span: Span,
    pub template: AttributeTemplate,
    pub path: AttrPath,
    pub reason: AttributeParseErrorReason<'a>,
    pub suggestions: Vec<String>,
}

impl<'a> AttributeParseError<'a> {
    fn render_expected_specific_argument<G: rustc_errors::EmissionGuarantee>(
        &self,
        diag: &mut Diag<'_, G>,
        possibilities: &[&str],
        strings: bool,
    ) {
        let quote = if strings { '"' } else { '`' };
        match possibilities {
            &[] => {}
            &[x] => {
                diag.span_label(
                    self.span,
                    format!("the only valid argument here is {quote}{x}{quote}"),
                );
            }
            [first, second] => {
                diag.span_label(
                    self.span,
                    format!("valid arguments are {quote}{first}{quote} or {quote}{second}{quote}"),
                );
            }
            [first @ .., second_to_last, last] => {
                let mut res = String::new();
                for i in first {
                    res.push_str(&format!("{quote}{i}{quote}, "));
                }
                res.push_str(&format!(
                    "{quote}{second_to_last}{quote} or {quote}{last}{quote}"
                ));

                diag.span_label(self.span, format!("valid arguments are {res}"));
            }
        }
    }

    fn render_expected_specific_argument_list<G: rustc_errors::EmissionGuarantee>(
        &self,
        diag: &mut Diag<'_, G>,
        possibilities: &[&str],
        strings: bool,
    ) {
        let description = "attribute";

        let quote = if strings { '"' } else { '`' };
        match possibilities {
            &[] => {}
            &[x] => {
                diag.span_label(
                    self.span,
                    format!(
                        "this {description} is only valid with {quote}{x}{quote} as an argument"
                    ),
                );
            }
            [first, second] => {
                diag.span_label(self.span, format!("this {description} is only valid with either {quote}{first}{quote} or {quote}{second}{quote} as an argument"));
            }
            [first @ .., second_to_last, last] => {
                let mut res = String::new();
                for i in first {
                    res.push_str(&format!("{quote}{i}{quote}, "));
                }
                res.push_str(&format!(
                    "{quote}{second_to_last}{quote} or {quote}{last}{quote}"
                ));

                diag.span_label(self.span, format!("this {description} is only valid with one of the following arguments: {res}"));
            }
        }
    }

    pub fn render_suggestions<G: rustc_errors::EmissionGuarantee>(&self, diag: &mut Diag<'_, G>) {
        let description = "attribute";

        diag.span_suggestions(
            self.span,
            if self.suggestions.len() == 1 {
                "must be of the form".to_string()
            } else {
                format!("try changing it to one of the valid forms of the {description}")
            },
            self.suggestions.iter().cloned(),
            Applicability::HasPlaceholders,
        );
    }
}

impl<'a, G: rustc_errors::EmissionGuarantee> Diagnostic<'a, G> for AttributeParseError<'a> {
    fn into_diag(
        self,
        dcx: rustc_errors::DiagCtxtHandle<'a>,
        level: rustc_errors::Level,
    ) -> Diag<'a, G> {
        let name = self.path.to_string();
        let description = "attribute";
        let mut diag = Diag::new(
            dcx,
            level,
            format!("malformed `{name}` {description} input"),
        );
        diag.span(self.span);
        {
            use AttributeParseErrorReason::*;
            match &self.reason {
                ExpectedNoArgs => {
                    diag.span_label(self.span, "didn't expect any arguments here");
                }
                ExpectedStringLiteral { byte_string } => {
                    if let Some(start_point_span) = byte_string {
                        diag.span_suggestion(
                            *start_point_span,
                            "consider removing the prefix",
                            "",
                            Applicability::MaybeIncorrect,
                        );
                        diag.note("expected a normal string literal, not a byte string literal");

                        // Avoid emitting an "attribute must be of the form" suggestion, as the
                        // attribute is likely to be well-formed already.
                        return diag;
                    }
                    diag.span_label(self.span, "expected a string literal here");
                }
                ExpectedIntegerLiteral => {
                    diag.span_label(self.span, "expected an integer literal here");
                }
                ExpectedIntegerLiteralInRange {
                    lower_bound,
                    upper_bound,
                } => {
                    diag.span_label(
                    self.span,
                    format!(
                        "expected an integer literal in the range of {lower_bound}..={upper_bound}"
                    ),
                );
                }
                ExpectedSingleArgument => {
                    diag.span_label(self.span, "expected a single argument here");
                }
                ExpectedArgument => {
                    diag.span_label(self.span, "expected an argument here");
                }
                ExpectedAtLeastOneArgument => {
                    diag.span_label(self.span, "expected at least 1 argument here");
                }
                ExpectedList => {
                    diag.span_label(self.span, "expected this to be a list");
                }
                ExpectedListOrNoArgs => {
                    diag.span_label(self.span, "expected a list or no arguments here");
                }
                ExpectedListWithNumArgsOrMore { args } => {
                    diag.span_label(self.span, format!("expected {args} or more items"));
                }
                ExpectedNameValueOrNoArgs => {
                    diag.span_label(self.span, "didn't expect a list here");
                }
                ExpectedNonEmptyStringLiteral => {
                    diag.span_label(self.span, "string is not allowed to be empty");
                }
                DuplicateKey(key) => {
                    diag.span_label(
                        self.span,
                        format!("found `{key}` used as a key more than once"),
                    );
                }
                ExpectedNotLiteral => {
                    diag.span_label(self.span, "didn't expect a literal here");
                }
                ExpectedNameValue(None) => {
                    // If the span is the entire attribute inner, the suggestion we add below this
                    // match already contains enough information.
                    diag.span_label(self.span, "expected this to be of the form `... = \"...\"`");
                }
                ExpectedNameValue(Some(name)) => {
                    diag.span_label(
                        self.span,
                        format!("expected this to be of the form `{name} = \"...\"`"),
                    );
                }
                MissingNameValue(name) => {
                    diag.span_label(self.span, format!("missing argument `{name} = \"...\"`"));
                }
                ExpectedSpecificArgument {
                    possibilities,
                    strings,
                    list: false,
                } => {
                    self.render_expected_specific_argument(&mut diag, possibilities, *strings);
                }
                ExpectedSpecificArgument {
                    possibilities,
                    strings,
                    list: true,
                } => {
                    self.render_expected_specific_argument_list(&mut diag, possibilities, *strings);
                }
                ExpectedIdentifier => {
                    diag.span_label(self.span, "expected a valid identifier here");
                }
            }
        }
        if let Some(link) = self.template.docs {
            diag.note(format!("for more information visit <{link}>"));
        }

        if self.suggestions.len() < 4 {
            self.render_suggestions(&mut diag);
        }
        diag
    }
}
