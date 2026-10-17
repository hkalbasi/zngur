use std::{marker::PhantomData, rc::Rc};

use rustc_ast::{MetaItem, MetaItemInner, MetaItemKind, MetaItemLit, tokenstream::TokenStream};
use rustc_attr_parsing::AttributeTemplate;
use rustc_session::parse::ParseSess;
use rustc_span::{ErrorGuaranteed, Span, Symbol};

use crate::{
    attrs::{
        Attr, AttrBuilder, AttrPath, AttrTarget, DynAttr,
        parse::{PartialParsedAttr, PartialParsedAttrKind, context::AcceptContext},
    },
    driver::with_ctxt,
    span_tools::{Spanned, respan},
};

#[derive(Debug)]
pub struct MetaItemListParser {
    pub span: Span,
    items: Vec<MetaItemOrLitParser>,
}

impl MetaItemListParser {
    pub fn new(
        psess: &ParseSess,
        tokens: TokenStream,
        span: Span,
    ) -> Result<Self, ErrorGuaranteed> {
        let mut parser = rustc_parse::parser::Parser::new(psess, tokens, None);

        match parser.parse_meta_seq_top() {
            Ok(items) => Ok(Self {
                span,
                items: items.into_iter().map(Into::into).collect(),
            }),
            Err(err) => Err(err.emit()),
        }
    }

    pub fn mixed(&self) -> impl Iterator<Item = &MetaItemOrLitParser> {
        self.items.iter()
    }

    #[allow(dead_code)] // part of the list parser's API
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[allow(dead_code)] // part of the list parser's API
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns Some if the list contains only a single element.
    ///
    /// Inside the Some is the parser to parse this single element.
    pub fn as_single(&self) -> Option<&MetaItemOrLitParser> {
        let mut iter = self.mixed();
        iter.next().filter(|_| iter.next().is_none())
    }
}

#[derive(Debug)]
pub enum MetaItemOrLitParser {
    MetaItemParser(MetaItemParser),
    Lit(MetaItemLit),
}

impl From<MetaItemInner> for MetaItemOrLitParser {
    fn from(value: MetaItemInner) -> Self {
        match value {
            MetaItemInner::MetaItem(item) => Self::MetaItemParser(MetaItemParser {
                path_span: item.path.span,
                span: item.span,
                path: item.path.segments.iter().map(|seg| seg.ident).into(),
                args: item.into(),
            }),
            MetaItemInner::Lit(lit) => Self::Lit(lit),
        }
    }
}

impl MetaItemOrLitParser {
    pub fn span(&self) -> Span {
        match self {
            MetaItemOrLitParser::MetaItemParser(meta_item_parser) => meta_item_parser.span(),
            MetaItemOrLitParser::Lit(meta_item_lit) => meta_item_lit.span,
        }
    }

    pub fn as_lit(&self) -> Option<&MetaItemLit> {
        match self {
            MetaItemOrLitParser::Lit(meta_item_lit) => Some(meta_item_lit),
            MetaItemOrLitParser::MetaItemParser(_) => None,
        }
    }

    pub fn meta_item(&self) -> Option<&MetaItemParser> {
        match self {
            MetaItemOrLitParser::MetaItemParser(parser) => Some(parser),
            MetaItemOrLitParser::Lit(_) => None,
        }
    }

    /// Returns some if this `MetaItemOrLitParser` is a `MetaItem` with no arguments
    pub fn meta_item_no_args(&self) -> Option<&MetaItemParser> {
        let meta_item = self.meta_item()?;
        match meta_item.args().as_no_args() {
            Ok(()) => Some(meta_item),
            Err(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct MetaItemParser {
    path_span: Span,
    span: Span,
    path: AttrPath,
    args: ArgParser,
}

impl MetaItemParser {
    /// For a single-segment meta item, returns its name; otherwise, returns `None`.
    #[allow(dead_code)] // part of the meta-item parser's API
    pub fn ident(&self) -> Option<String> {
        if let [ident] = self.path.segments().collect::<Vec<_>>()[..] {
            Some(ident.to_string())
        } else {
            None
        }
    }

    pub fn span(&self) -> Span {
        if let Some(other) = self.args.span() {
            self.span.with_hi(other.hi())
        } else {
            self.span
        }
    }

    /// Gets just the path, without the args.
    pub fn path(&self) -> &AttrPath {
        &self.path
    }

    /// Gets just the args parser, without caring about the path.
    pub fn args(&self) -> &ArgParser {
        &self.args
    }

    /// Asserts that this `MetaItem` starts with a word, or single segment path.
    #[allow(dead_code)] // part of the meta-item parser's API
    pub fn word_is(&self, s: &str) -> Option<&ArgParser> {
        self.path()
            .as_ident()
            .filter(|ident| *ident == s)
            .is_some()
            .then(|| self.args())
    }
}

#[derive(Clone, Debug)]
pub struct NameValueParser {
    pub eq_span: Span,
    pub value_span: Span,
    value: MetaItemLit,
}

impl NameValueParser {
    pub fn value_as_lit(&self) -> &MetaItemLit {
        &self.value
    }

    pub fn value_as_str(&self) -> Option<Symbol> {
        self.value_as_lit().kind.str()
    }
    pub fn args_span(&self) -> Span {
        self.eq_span.to(self.value_span)
    }
}

#[derive(Debug)]
pub enum ArgParser {
    NoArgs,
    List(MetaItemListParser),
    NameValue(NameValueParser),
}

impl ArgParser {
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::NoArgs => None,
            Self::List(l) => Some(l.span),
            Self::NameValue(n) => Some(n.args_span()),
        }
    }

    pub fn as_list(&self) -> Option<&MetaItemListParser> {
        match self {
            Self::List(l) => Some(l),
            Self::NameValue(_) | Self::NoArgs => None,
        }
    }

    pub fn as_name_value(&self) -> Option<&NameValueParser> {
        match self {
            Self::NameValue(n) => Some(n),
            Self::List(_) | Self::NoArgs => None,
        }
    }

    pub fn as_no_args(&self) -> Result<(), Span> {
        match self {
            Self::NoArgs => Ok(()),
            Self::List(args) => Err(args.span),
            Self::NameValue(args) => Err(args.args_span()),
        }
    }
}

pub trait ExpectNameValue {
    type Output<'a>
    where
        Self: 'a;
    fn expect_name_value<'a, 'f, 'sess>(
        &'a self,
        cx: &mut AcceptContext<'f, 'sess>,
        span: Span,
        name: Option<&'a str>,
    ) -> Option<Self::Output<'a>>;
}

impl ExpectNameValue for MetaItemOrLitParser {
    type Output<'a> = (Spanned<&'a str>, &'a NameValueParser);

    fn expect_name_value<'a, 'f, 'sess>(
        &'a self,
        cx: &mut AcceptContext<'f, 'sess>,
        span: Span,
        name: Option<&'a str>,
    ) -> Option<Self::Output<'a>> {
        let Some(meta_item) = self.meta_item() else {
            cx.expected_name_value(self.span(), name);
            return None;
        };

        meta_item.expect_name_value(cx, span, name)
    }
}

impl ExpectNameValue for MetaItemParser {
    type Output<'a> = (Spanned<&'a str>, &'a NameValueParser);

    fn expect_name_value<'a, 'f, 'sess>(
        &'a self,
        cx: &mut AcceptContext<'f, 'sess>,
        _span: Span, // Not needed: `MetaItemOrLitParser` carry its own span.
        name: Option<&'a str>,
    ) -> Option<Self::Output<'a>> {
        let Some(word) = self.path().as_ident() else {
            cx.expected_identifier(self.path_span);
            return None;
        };
        let Some(arg) = self.args().as_name_value() else {
            cx.expected_name_value(self.span(), name);
            return None;
        };

        Some((respan(self.span.until(arg.args_span()), word), arg))
    }
}

impl ExpectNameValue for ArgParser {
    type Output<'a> = &'a NameValueParser;

    fn expect_name_value<'a, 'f, 'sess>(
        &'a self,
        cx: &mut AcceptContext<'f, 'sess>,
        span: Span,
        name: Option<&'a str>,
    ) -> Option<Self::Output<'a>> {
        let Some(nv) = self.as_name_value() else {
            cx.expected_name_value(span, name);
            return None;
        };

        Some(nv)
    }
}

pub trait ExpectStringLiteral {
    fn expect_string_literal<'f, 'sess>(
        &self,
        cx: &mut AcceptContext<'f, 'sess>,
    ) -> Option<Spanned<String>>;
}

impl ExpectStringLiteral for NameValueParser {
    fn expect_string_literal<'f, 'sess>(
        &self,
        cx: &mut AcceptContext<'f, 'sess>,
    ) -> Option<Spanned<String>> {
        let Some(value) = self.value_as_str().map(|s| s.to_string()) else {
            cx.expected_string_literal(self.value_span, Some(self.value_as_lit()));
            return None;
        };
        Some(respan(self.value_span, value))
    }
}

impl ExpectStringLiteral for MetaItemOrLitParser {
    fn expect_string_literal<'f, 'sess>(
        &self,
        cx: &mut AcceptContext<'f, 'sess>,
    ) -> Option<Spanned<String>> {
        let Some(lit) = self.as_lit() else {
            cx.expected_string_literal(self.span(), None);
            return None;
        };

        let Some(value) = lit.value_as_str().map(|s| s.to_string()) else {
            cx.expected_string_literal(self.span(), Some(lit));
            return None;
        };

        Some(respan(lit.span, value))
    }
}

impl ExpectStringLiteral for MetaItemLit {
    fn expect_string_literal<'f, 'sess>(
        &self,
        cx: &mut AcceptContext<'f, 'sess>,
    ) -> Option<Spanned<String>> {
        let Some(value) = self.value_as_str().map(|s| s.to_string()) else {
            cx.expected_string_literal(self.span, Some(self));
            return None;
        };

        Some(respan(self.span, value))
    }
}

pub trait ExpectIdent {
    fn expect_ident<'f, 'sess>(&self, cx: &mut AcceptContext<'f, 'sess>) -> Option<&str>;
}

impl ExpectIdent for MetaItemOrLitParser {
    fn expect_ident<'f, 'sess>(&self, cx: &mut AcceptContext<'f, 'sess>) -> Option<&str> {
        let Some(item) = self.meta_item_no_args() else {
            cx.expected_identifier(self.span());
            return None;
        };

        let ident = item.path().as_ident();
        if ident.is_none() {
            cx.expected_identifier(self.span());
        }

        ident
    }
}

impl From<MetaItem> for ArgParser {
    fn from(value: MetaItem) -> Self {
        match value.kind {
            MetaItemKind::Word => ArgParser::NoArgs,
            MetaItemKind::List(items) => ArgParser::List(MetaItemListParser {
                span: value.span,
                items: items.into_iter().map(Into::into).collect(),
            }),
            MetaItemKind::NameValue(lit) => ArgParser::NameValue(NameValueParser {
                eq_span: value.path.span.between(lit.span),
                value_span: lit.span,
                value: lit,
            }),
        }
    }
}

pub type AcceptFn<T> = for<'sess> fn(&mut T, &mut AcceptContext<'_, 'sess>, &ArgParser);
pub type AcceptAttrMap<T> = &'static [(AttrPath, AttributeTemplate, AcceptFn<T>)];

#[allow(dead_code)] // used by the `ASSERTION` below; kept as part of the framework
pub const MAX_PARSER_ATTRS: usize = 4;
pub trait AttrtibuteParser: Default + 'static {
    type Attr: Attr;
    const ATTRIBUTES: AcceptAttrMap<Self>;

    fn finalize(self, target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr>;
}

#[derive(Default)]
pub struct ParserBuilder<T: AttrtibuteParser>(PhantomData<T>);

impl<T: AttrtibuteParser> ParserBuilder<T> {
    pub const fn new() -> Self {
        ParserBuilder(PhantomData)
    }
}

impl<T: AttrtibuteParser> std::fmt::Debug for ParserBuilder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParserBuilder<")?;
        f.write_str(std::any::type_name::<T>())?;
        f.write_str(">")
    }
}

impl<T: AttrtibuteParser> ParserBuilder<T> {
    #[allow(dead_code)] // compile-time sanity check, not referenced at runtime
    const ASSERTION: () = assert!(MAX_PARSER_ATTRS >= T::ATTRIBUTES.len());
}

impl<T: AttrtibuteParser> AttrBuilder for ParserBuilder<T> {
    fn valid_for(&self, path: &AttrPath) -> bool {
        T::ATTRIBUTES.iter().find(|(p, _, _)| p == path).is_some()
    }

    fn paths(&self) -> Vec<AttrPath> {
        T::ATTRIBUTES.iter().map(|(p, _, _)| p).cloned().collect()
    }

    fn parse(&self, attr: PartialParsedAttr, item: &dyn AttrTarget) -> Option<Rc<dyn DynAttr>> {
        let Some((path, template, accept)) =
            T::ATTRIBUTES.iter().find(|(path, _, _)| attr.path == *path)
        else {
            with_ctxt(|ctx| {
                ctx.tcx.dcx().struct_err(
                  format!("BUG! parsing the attribute `{}` with a parser not configured for it: `{self:?}`", attr.path)
              ).emit();
            });
            return None;
        };

        let item = item.cast_ref::<<T::Attr as Attr>::Target>()?;

        let mut parser = T::default();
        with_ctxt(|ctxt| -> Result<_, ErrorGuaranteed> {
            let mut cx = AcceptContext {
                sess: ctxt.tcx.sess,
                dcx: ctxt.tcx.dcx(),
                template,
                attr_path: path,
                span: attr.span,
            };
            let arg_parser = match attr.kind {
                PartialParsedAttrKind::Path => ArgParser::NoArgs,
                PartialParsedAttrKind::List { args } => ArgParser::List(MetaItemListParser::new(
                    &ctxt.tcx.sess.psess,
                    args.tokens,
                    args.dspan.entire(),
                )?),
                PartialParsedAttrKind::NameValue { args } => {
                    ArgParser::NameValue(NameValueParser {
                        eq_span: args.eq_span,
                        value_span: args.span,
                        value: args.lit,
                    })
                }
            };
            accept(&mut parser, &mut cx, &arg_parser);
            Ok(())
        })
        .ok()?;

        T::finalize(parser, item).map(|attr| Rc::new(attr) as Rc<dyn DynAttr>)
    }
}
