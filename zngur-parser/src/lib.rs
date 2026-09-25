use std::{
    collections::HashSet,
    fmt::Display,
    ops::{Deref, DerefMut},
    path::Component,
};

use ariadne::{Color, Label, Report, ReportKind};
use chumsky::{input::MapExtra, prelude::*};
use itertools::{Either, Itertools};

use zngur_def::{
    AdditionalIncludes, ConflictSource, ConvertPanicToException, CppHeapAllocated, CppRef,
    CppStackOwned, Import, LayoutPolicy, Merge, MergeFailure, ModuleImport, Mutability,
    PrimitiveRustType, RustPathAndGenerics, RustTrait, RustType, TypeVar, ZngurConstructor,
    ZngurExternCppFn, ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod, ZngurMethodDetails,
    ZngurMethodReceiver, ZngurSpec, ZngurTrait, ZngurType, ZngurVariant, ZngurWellknownTrait,
};

pub type Span = SimpleSpan<usize>;

/// Result of parsing a .zng file, containing both the spec and the list of all processed files.
#[derive(Debug)]
pub struct ParseResult {
    /// The parsed Zngur specification
    pub spec: ZngurSpec,
    /// All .zng files that were processed (main file + transitive imports)
    pub processed_files: Vec<std::path::PathBuf>,
    /// count of errors reported
    pub errors: usize,
    /// count of warnings reported
    pub warnings: usize,
}

#[cfg(test)]
mod tests;

pub mod cfg;
mod conditional;
mod template_types;

use crate::{
    cfg::{CfgConditional, RustCfgProvider},
    conditional::{Condition, ConditionalItem, NItems, conditional_item},
    template_types::try_match_template,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spanned<T> {
    inner: T,
    span: Span,
}

type ParserInput<'a> = chumsky::input::MappedInput<
    Token<'a>,
    Span,
    &'a [(Token<'a>, Span)],
    Box<
        dyn for<'x> Fn(
            &'x (Token<'_>, chumsky::span::SimpleSpan),
        ) -> (&'x Token<'x>, &'x SimpleSpan),
    >,
>;

#[derive(Default)]
pub struct UnstableFeatures {
    pub cfg_match: bool,
    pub cfg_if: bool,
    pub template_types: bool,
}

#[derive(Default)]
pub struct ZngParserState {
    pub unstable_features: UnstableFeatures,
}

type ZngParserExtra<'a> =
    extra::Full<Rich<'a, Token<'a>, Span>, extra::SimpleState<ZngParserState>, ()>;

type BoxedZngParser<'a, Item> = chumsky::Boxed<'a, 'a, ParserInput<'a>, Item, ZngParserExtra<'a>>;

/// Effective trait alias for verbose chumsky Parser Trait
pub(crate) trait ZngParser<'a, Item>:
    Parser<'a, ParserInput<'a>, Item, ZngParserExtra<'a>> + Clone
{
}
impl<'a, T, Item> ZngParser<'a, Item> for T where
    T: Parser<'a, ParserInput<'a>, Item, ZngParserExtra<'a>> + Clone
{
}

#[derive(Debug)]
pub struct ParsedZngFile<'a>(Vec<ParsedItem<'a>>);

#[derive(Debug)]
pub struct ProcessedZngFile<'a> {
    aliases: Vec<ParsedAlias<'a>>,
    items: Vec<ProcessedItem<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ParsedPathStart {
    Absolute,
    Relative,
    Crate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPath<'a> {
    start: ParsedPathStart,
    segments: Vec<&'a str>,
    span: Span,
}

#[derive(Debug, Clone)]
struct Scope<'a> {
    aliases: Vec<ParsedAlias<'a>>,
    base: Vec<String>,
    type_vars: HashSet<ParsedTypeVar<'a>>,
}

impl<'a> Scope<'a> {
    /// Create a new root scope containing the specified aliases.
    fn new_root(aliases: Vec<ParsedAlias<'a>>) -> Scope<'a> {
        Scope {
            aliases,
            base: Default::default(),
            type_vars: Default::default(),
        }
    }

    /// Resolve a path according to the current scope.
    fn resolve_path(&self, path: ParsedPath<'a>) -> Vec<String> {
        // Check to see if the path refers to an alias:
        if let Some(expanded_alias) = self
            .aliases
            .iter()
            .find_map(|alias| alias.expand(&path, &self.base))
        {
            expanded_alias
        } else {
            path.to_zngur(&self.base)
        }
    }

    /// Create a fully-qualified path relative to this scope's base path.
    fn simple_relative_path(&self, relative_item_name: &str) -> Vec<String> {
        self.base
            .iter()
            .cloned()
            .chain(Some(relative_item_name.to_string()))
            .collect()
    }

    fn sub_scope(&self, new_aliases: &[ParsedAlias<'a>], nested_path: ParsedPath<'a>) -> Scope<'_> {
        let base = nested_path.to_zngur(&self.base);
        let mut mod_aliases = new_aliases.to_vec();
        mod_aliases.extend_from_slice(&self.aliases);

        Scope {
            aliases: mod_aliases,
            base,
            type_vars: self.type_vars.clone(),
        }
    }

    fn with_type_vars(&self, type_vars: HashSet<ParsedTypeVar<'a>>) -> Scope<'_> {
        Scope {
            aliases: self.aliases.clone(),
            base: self.base.clone(),
            type_vars,
        }
    }

    fn as_type_var(&self, ty: &ParsedRustPathAndGenerics<'a>) -> Option<TypeVar> {
        if let ParsedRustPathAndGenerics {
            path:
                ParsedPath {
                    start: ParsedPathStart::Relative,
                    segments,
                    span: _,
                },
            generics,
            named_generics,
        } = ty
            && generics.is_empty()
            && named_generics.is_empty()
            && let &[single_elem] = segments.as_slice()
            && self.type_vars.contains(&ParsedTypeVar(single_elem))
        {
            Some(TypeVar(single_elem.to_owned()))
        } else {
            None
        }
    }
}

impl ParsedPath<'_> {
    fn to_zngur(self, base: &[String]) -> Vec<String> {
        match self.start {
            ParsedPathStart::Absolute => self.segments.into_iter().map(|x| x.to_owned()).collect(),
            ParsedPathStart::Relative => base
                .iter()
                .map(|x| x.as_str())
                .chain(self.segments)
                .map(|x| x.to_owned())
                .collect(),
            ParsedPathStart::Crate => ["crate"]
                .into_iter()
                .chain(self.segments)
                .map(|x| x.to_owned())
                .collect(),
        }
    }

    fn matches_alias(&self, alias: &ParsedAlias<'_>) -> bool {
        match self.start {
            ParsedPathStart::Absolute | ParsedPathStart::Crate => false,
            ParsedPathStart::Relative => self
                .segments
                .first()
                .is_some_and(|part| *part == alias.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlias<'a> {
    name: &'a str,
    path: ParsedPath<'a>,
    span: Span,
}

impl ParsedAlias<'_> {
    fn expand(&self, path: &ParsedPath<'_>, base: &[String]) -> Option<Vec<String>> {
        if path.matches_alias(self) {
            match self.path.start {
                ParsedPathStart::Absolute => Some(
                    self.path
                        .segments
                        .iter()
                        .chain(path.segments.iter().skip(1))
                        .map(|seg| (*seg).to_owned())
                        .collect(),
                ),
                ParsedPathStart::Crate => Some(
                    ["crate"]
                        .into_iter()
                        .chain(self.path.segments.iter().cloned())
                        .chain(path.segments.iter().skip(1).cloned())
                        .map(|seg| (*seg).to_owned())
                        .collect(),
                ),
                ParsedPathStart::Relative => Some(
                    base.iter()
                        .map(|x| x.as_str())
                        .chain(self.path.segments.iter().cloned())
                        .chain(path.segments.iter().skip(1).cloned())
                        .map(|seg| (*seg).to_owned())
                        .collect(),
                ),
            }
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedImportPath {
    path: std::path::PathBuf,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedItem<'a> {
    ConvertPanicToException(Span),
    CppAdditionalInclude(&'a str),
    UnstableFeature(&'a str),
    Mod {
        path: ParsedPath<'a>,
        items: Vec<ParsedItem<'a>>,
    },
    Type {
        ty: Spanned<ParsedRustType<'a>>,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<ParsedMethod<'a>>,
    },
    Fn(Spanned<ParsedMethod<'a>>),
    ExternCpp(Vec<ParsedExternCppItem<'a>>),
    Alias(ParsedAlias<'a>),
    Import(ParsedImportPath),
    ModuleImport {
        path: std::path::PathBuf,
        span: Span,
    },
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProcessedItem<'a> {
    ConvertPanicToException(Span),
    CppAdditionalInclude(&'a str),
    Mod {
        path: ParsedPath<'a>,
        items: Vec<ProcessedItem<'a>>,
        aliases: Vec<ParsedAlias<'a>>,
    },
    Type {
        ty: Spanned<ParsedRustType<'a>>,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<ParsedMethod<'a>>,
    },
    Fn(Spanned<ParsedMethod<'a>>),
    ExternCpp(Vec<ParsedExternCppItem<'a>>),
    Import(ParsedImportPath),
    ModuleImport {
        path: std::path::PathBuf,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedExternCppItem<'a> {
    Function {
        is_safe: bool,
        method: Spanned<ParsedMethod<'a>>,
    },
    Impl {
        tr: Option<ParsedRustTrait<'a>>,
        ty: Spanned<ParsedRustType<'a>>,
        methods: Vec<(bool, ParsedMethod<'a>)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedConstructorArgs<'a> {
    Unit,
    Tuple(Vec<ParsedRustType<'a>>),
    Named(Vec<(&'a str, ParsedRustType<'a>)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedLayoutPolicy<'a> {
    StackAllocated(Vec<(Spanned<&'a str>, usize)>),
    Conservative(Vec<(Spanned<&'a str>, usize)>),
    HeapAllocated,
    OnlyByRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedTypeItem<'a> {
    Layout(Span, ParsedLayoutPolicy<'a>),
    Traits(Vec<Spanned<ZngurWellknownTrait>>),
    NonExhaustive,
    Constructor {
        args: ParsedConstructorArgs<'a>,
    },
    Variant {
        name: &'a str,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
    },
    Field {
        name: String,
        ty: ParsedRustType<'a>,
        offset: Option<usize>,
    },
    Method {
        data: ParsedMethod<'a>,
        use_path: Option<ParsedPath<'a>>,
        deref: Option<ParsedRustType<'a>>,
        cpp_name: Option<&'a str>,
    },
    CppValue {
        field: &'a str,
        cpp_type: &'a str,
    },
    CppHeapAllocated {
        cpp_type: &'a str,
    },
    CppRef {
        cpp_type: &'a str,
    },
    CppStackOwned {
        cpp_type: &'a str,
        props: Vec<(Spanned<&'a str>, usize)>,
    },
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedTypeItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParsedTypeVar<'a>(&'a str);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMethod<'a> {
    name: &'a str,
    receiver: ZngurMethodReceiver,
    generics: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
}

impl ParsedMethod<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> ZngurMethod {
        ZngurMethod {
            name: self.name.to_owned(),
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope))
                .collect(),
            receiver: self.receiver,
            inputs: self.inputs.into_iter().map(|x| x.to_zngur(scope)).collect(),
            output: self.output.to_zngur(scope),
            is_safe: true,
        }
    }
}

struct MergeContext {
    outer_ty: RustType,
    variant: Option<String>,
}

fn checked_merge<T, U>(
    src: T,
    dst: &mut U,
    span: Span,
    ctx: &mut ParseContext,
    src_ctx: Option<MergeContext>,
) where
    T: Merge<U>,
{
    match src.merge(dst) {
        Ok(()) => {}
        Err(e) => match e {
            MergeFailure::Conflict(s, conflict) => {
                ctx.add_fatal_report(build_merge_conflict_report(
                    ctx, span, &s, conflict, src_ctx,
                ));
            }
        },
    }
}

fn build_template_conflict_report(
    ctx: &ParseContext,
    template: &TemplateDef,
    target_ty: &ZngurType,
    template_span: ReportSpan,
    msg: &str,
    conflict: (ConflictSource, ConflictSource),
) -> ParseReport<'static> {
    let spans = ctx.fetch_spans_global(target_ty);
    let (first, rest) = {
        let mut it = spans.into_iter();
        let first = it.next().cloned();
        (first, it.collect::<Vec<_>>())
    };

    let mut report = Report::build(ReportKind::Error, (0usize, 0usize..0))
        .with_message(format!(
            "Failed to apply template {} to type {}: {msg}",
            template.ty.ty, target_ty.ty
        ))
        .with_label(
            Label::new(template_span)
                .with_message("Template declared here")
                .with_color(Color::Blue),
        );

    if let Some(first) = first {
        report.add_label(
            Label::new(first)
                .with_message("Type first declared here.")
                .with_color(Color::Blue),
        );
    }

    add_conflict_labels(
        ctx,
        &mut report,
        conflict,
        &MergeContext {
            outer_ty: target_ty.ty.clone(),
            variant: None,
        },
    );

    let count = rest.len();
    for (i, span) in rest.into_iter().enumerate() {
        report.add_label(
            Label::new(span.clone())
                .with_message("Type also declared here")
                .with_color(Color::Blue),
        );
        if i >= 2 {
            report.add_note(format!(
                "{} additional type declaration sites omitted",
                count - i
            ));
            break;
        }
    }
    report.finish()
}

fn build_merge_conflict_report(
    ctx: &ParseContext,
    span: Span,
    msg: &str,
    conflict: (ConflictSource, ConflictSource),
    merge_ctx: Option<MergeContext>,
) -> ParseReport<'static> {
    let report_span = ctx.report_span_for_range(span.into_range());
    let mut report = Report::build(ReportKind::Error, report_span.clone())
        .with_message(msg)
        .with_label(
            Label::new(report_span)
                .with_message(msg)
                .with_color(Color::Red),
        );
    if let Some(merge_ctx) = &merge_ctx {
        add_conflict_labels(ctx, &mut report, conflict, merge_ctx);
    }
    report.finish()
}

fn add_conflict_labels(
    ctx: &ParseContext,
    report: &mut ariadne::ReportBuilder<'static, ReportSpan>,
    conflict: (ConflictSource, ConflictSource),
    merge_ctx: &MergeContext,
) {
    let (src_conflict, dst_conflict) = conflict;
    let src_span_key = src_conflict
        .clone()
        .into_span_key_with_variant(merge_ctx.outer_ty.clone(), merge_ctx.variant.clone());
    let dst_span_key = dst_conflict
        .clone()
        .into_span_key_with_variant(merge_ctx.outer_ty.clone(), merge_ctx.variant.clone());
    let src_spans = ctx.fetch_spans_global(&src_span_key);
    let dst_spans = ctx.fetch_spans_global(&dst_span_key);
    let (src_span, dst_span, first_span, rest) = if src_conflict == dst_conflict {
        let mut src = src_spans.into_iter();
        (
            src.next_back().cloned(),
            src.next_back().cloned(),
            src.next().cloned(),
            src.collect::<Vec<_>>(),
        )
    } else {
        let mut src = src_spans.into_iter();
        let mut dst = dst_spans.into_iter();
        (
            src.next_back().cloned(),
            dst.next_back().cloned(),
            dst.next().cloned(),
            dst.collect::<Vec<_>>(),
        )
    };
    if let Some(src_span) = src_span {
        report.add_label(
            Label::new(src_span)
                .with_message("declaration here ...")
                .with_color(Color::Yellow),
        );
    }
    if let Some(dst_span) = dst_span {
        report.add_label(
            Label::new(dst_span)
                .with_message("conflicts with the declaration here.")
                .with_color(Color::Yellow),
        );
    }
    if let Some(first_span) = first_span {
        report.add_label(
            Label::new(first_span)
                .with_message("first declared here.")
                .with_color(Color::Yellow),
        );
    }
    let count = rest.len();
    for (i, span) in rest.into_iter().enumerate() {
        report.add_label(
            Label::new(span.clone())
                .with_message("also declared here.")
                .with_color(Color::Blue),
        );
        if i >= 2 {
            report.add_note(format!("{} additional conflict sites omitted", count - i));
        }
    }
}

impl ProcessedItem<'_> {
    fn add_to_zngur_spec(
        self,
        r: &mut ZngurSpecBuilder,
        scope: &Scope<'_>,
        ctx: &mut ParseContext,
    ) {
        match self {
            ProcessedItem::Mod {
                path,
                items,
                aliases,
            } => {
                let sub_scope = scope.sub_scope(&aliases, path);
                for item in items {
                    item.add_to_zngur_spec(r, &sub_scope, ctx);
                }
            }
            ProcessedItem::Import(path) => {
                if path.path.is_absolute() {
                    ctx.add_error_str("Absolute paths imports are not supported.", path.span)
                }
                match path.path.components().next() {
                    Some(Component::CurDir) | Some(Component::ParentDir) => {
                        let import = Import(path.path);
                        ctx.record_span(&import, path.span.into_range());
                        r.imports.push(import);
                    }
                    _ => ctx.add_error_str(
                        "Module import is not supported. Use a relative path instead.",
                        path.span,
                    ),
                }
            }
            ProcessedItem::ModuleImport { path, span: _ } => {
                r.spec
                    .imported_modules
                    .push(ModuleImport { path: path.clone() });
            }
            ProcessedItem::Type {
                ty,
                items,
                type_vars,
            } => {
                if ty.inner == ParsedRustType::Tuple(vec![]) {
                    // We add unit type implicitly.
                    ctx.add_error_str(
                        "Unit type is declared implicitly. Remove this entirely.",
                        ty.span,
                    );
                }

                let (is_template, scope) = match type_vars {
                    Some(type_vars) => (true, &scope.with_type_vars(type_vars)),
                    None => (false, scope),
                };

                let mut methods = vec![];
                let mut constructor = None;
                let mut variants = vec![];
                let mut fields = vec![];
                let mut wellknown_traits = vec![];
                let mut layout = None;
                let mut layout_span = None;
                let mut exhaustive = true;
                let mut cpp_heap_allocated = None;
                let mut cpp_ref = None;
                let mut cpp_stack_owned = None;
                let mut to_process = items;
                to_process.reverse(); // create a stack of items to process
                let check_size_align = |props: Vec<(Spanned<&str>, usize)>| {
                    let mut size = None;
                    let mut align = None;
                    let mut errors = vec![];
                    for (key, value) in props {
                        match key.inner {
                            "size" => size = Some(value),
                            "align" => align = Some(value),
                            _ => errors.push(("Unknown property", key.span)),
                        }
                    }
                    if size.is_none() {
                        errors.push(("Size is not declared for this type", ty.span));
                    }
                    if align.is_none() {
                        errors.push(("Align is not declared for this type", ty.span));
                    }
                    if errors.is_empty() {
                        Ok((size.unwrap(), align.unwrap()))
                    } else {
                        Err(errors)
                    }
                };
                let rust_ty = ty.inner.to_zngur(scope);
                while let Some(item) = to_process.pop() {
                    let item_span = item.span;
                    let item = item.inner;
                    match item {
                        ParsedTypeItem::Layout(span, p) => {
                            let l = match p {
                                ParsedLayoutPolicy::StackAllocated(p) => {
                                    match check_size_align(p) {
                                        Ok((size, align)) => {
                                            LayoutPolicy::StackAllocated { size, align }
                                        }
                                        Err(errs) => {
                                            for (msg, span) in errs {
                                                ctx.add_error_str(msg, span);
                                            }
                                            continue;
                                        }
                                    }
                                }
                                ParsedLayoutPolicy::Conservative(p) => match check_size_align(p) {
                                    Ok((size, align)) => LayoutPolicy::Conservative { size, align },
                                    Err(errs) => {
                                        for (msg, span) in errs {
                                            ctx.add_error_str(msg, span);
                                        }
                                        continue;
                                    }
                                },
                                ParsedLayoutPolicy::HeapAllocated => LayoutPolicy::HeapAllocated,
                                ParsedLayoutPolicy::OnlyByRef => LayoutPolicy::OnlyByRef,
                            };
                            ctx.record_span(
                                &l.into_span_key_with(rust_ty.clone()),
                                span.into_range(),
                            );
                            layout = Some(l);
                            match layout_span {
                                Some(_) => {
                                    ctx.add_error_str("Duplicate layout policy found", span);
                                }
                                None => layout_span = Some(span),
                            }
                        }
                        ParsedTypeItem::Traits(tr) => {
                            wellknown_traits.extend(tr);
                        }
                        ParsedTypeItem::NonExhaustive => {
                            if !exhaustive {
                                ctx.add_error_str(
                                    "Duplicate non_exhaustive annotation found",
                                    item_span,
                                );
                            }
                            exhaustive = false;
                        }
                        ParsedTypeItem::Constructor { args } => {
                            if constructor.is_some() {
                                ctx.add_error_str("Duplicate constructor found", item_span);
                            }
                            let c = ZngurConstructor {
                                inputs: match args {
                                    ParsedConstructorArgs::Unit => vec![],
                                    ParsedConstructorArgs::Tuple(t) => t
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, t)| (i.to_string(), t.to_zngur(scope)))
                                        .collect(),
                                    ParsedConstructorArgs::Named(t) => t
                                        .into_iter()
                                        .map(|(i, t)| (i.to_owned(), t.to_zngur(scope)))
                                        .collect(),
                                },
                            };
                            ctx.record_span(
                                &c.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            constructor = Some(c);
                        }
                        ParsedTypeItem::Variant { name, items } => {
                            let mut exhaustive = true;
                            let mut fields = vec![];
                            for item in items {
                                match item.inner {
                                    ParsedTypeItem::NonExhaustive => {
                                        if !exhaustive {
                                            ctx.add_error_str(
                                                "Duplicate non_exhaustive annotation found",
                                                item.span,
                                            );
                                        }
                                        exhaustive = false;
                                    }
                                    ParsedTypeItem::Field { name, ty, offset } => {
                                        if offset.is_some() {
                                            ctx.add_error_str(
                                                "static offsets on enum fields are not supported",
                                                item.span,
                                            );
                                        }
                                        let field = ZngurField {
                                            name: name.to_owned(),
                                            ty: ty.to_zngur(scope),
                                            offset,
                                        };
                                        ctx.record_span(
                                            &field.into_span_key_with_variant(
                                                rust_ty.clone(),
                                                Some(name.clone()),
                                            ),
                                            item.span.into_range(),
                                        );
                                        fields.push(field);
                                    }
                                    _ => panic!("bug: invalid variant item found: {item:?}"),
                                }
                            }
                            variants.push(ZngurVariant {
                                name: name.to_owned(),
                                fields,
                                exhaustive,
                            });
                        }
                        ParsedTypeItem::Field { name, ty, offset } => {
                            let field = ZngurField {
                                name: name.to_owned(),
                                ty: ty.to_zngur(scope),
                                offset,
                            };
                            ctx.record_span(
                                &field.into_span_key_with_variant(rust_ty.clone(), None),
                                item_span.into_range(),
                            );
                            fields.push(field);
                        }
                        ParsedTypeItem::Method {
                            data,
                            use_path,
                            deref,
                            cpp_name,
                        } => {
                            let deref = deref.and_then(|x| {
                                let deref_type = x.to_zngur(scope);
                                let receiver_mutability = match data.receiver {
                                    ZngurMethodReceiver::Ref(mutability) => mutability,
                                    ZngurMethodReceiver::Static | ZngurMethodReceiver::Move => {
                                        ctx.add_error_str(
                                            "Deref needs reference receiver",
                                            item_span,
                                        );
                                        return None;
                                    }
                                };
                                Some((deref_type, receiver_mutability))
                            });
                            let method = ZngurMethodDetails {
                                data: data.to_zngur(scope),
                                use_path: use_path.map(|x| scope.resolve_path(x)),
                                deref,
                                cpp_name: cpp_name.map(|s| s.to_owned()),
                            };
                            ctx.record_span(
                                &method.data.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            methods.push(method);
                        }
                        ParsedTypeItem::CppValue { field: _, cpp_type } => {
                            ctx.add_warning_str(
                                "#cpp_value is deprecated; use #cpp_heap_allocated instead",
                                item_span,
                            );
                            let cpp = CppHeapAllocated(cpp_type.to_owned());
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppHeapAllocated { cpp_type } => {
                            let cpp = CppHeapAllocated(cpp_type.to_owned());
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppRef { cpp_type } => {
                            match layout_span {
                                Some(span) => {
                                    ctx.add_error_str("Duplicate layout policy found", span);
                                    continue;
                                }
                                None => {
                                    layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                                    layout_span = Some(item_span);
                                }
                            }
                            let cpp = CppRef(cpp_type.to_owned());
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_ref = Some(cpp);
                        }
                        ParsedTypeItem::CppStackOwned { cpp_type, props } => {
                            let (size, align) = match check_size_align(props) {
                                Ok(x) => x,
                                Err(errs) => {
                                    for (msg, span) in errs {
                                        ctx.add_error_str(msg, span);
                                    }
                                    continue;
                                }
                            };
                            let cpp = CppStackOwned {
                                cpp_type: cpp_type.to_owned(),
                                size,
                                align,
                            };
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_stack_owned = Some(cpp);
                            layout = Some(LayoutPolicy::StackAllocated { size, align });
                        }
                        ParsedTypeItem::MatchOnCfg(match_) => {
                            let result = match_.eval(ctx);
                            if let Some(result) = result {
                                to_process.extend(result);
                            }
                        }
                    }
                }
                let is_unsized = wellknown_traits
                    .iter()
                    .find(|x| x.inner == ZngurWellknownTrait::Unsized)
                    .cloned();
                let wt = wellknown_traits
                    .into_iter()
                    .map(|x| x.inner)
                    .collect::<Vec<_>>();
                if let Some(is_unsized) = is_unsized {
                    if let Some(span) = layout_span {
                        ctx.add_fatal_report(
                            Report::build(
                                ReportKind::Error,
                                ctx.report_span_for_range(span.into_range()),
                            )
                            .with_message("Duplicate layout policy found for unsized type.")
                            .with_label(
                                Label::new(ctx.report_span_for_range(span.into_range()))
                                    .with_message(
                                        "Unsized types have implicit layout policy, remove this.",
                                    )
                                    .with_color(Color::Red),
                            )
                            .with_label(
                                Label::new(ctx.report_span_for_range(is_unsized.span.into_range()))
                                    .with_message("Type declared as unsized here.")
                                    .with_color(Color::Blue),
                            )
                            .finish(),
                        )
                    }
                    layout = Some(LayoutPolicy::OnlyByRef);
                }
                let zngur_type = ZngurType {
                    ty: rust_ty.clone(),
                    layout,
                    methods,
                    wellknown_traits: wt,
                    exhaustive,
                    constructor,
                    variants,
                    fields,
                    cpp_heap_allocated,
                    cpp_ref,
                    cpp_stack_owned,
                };
                if is_template {
                    r.templates.push(TemplateDef {
                        ty: zngur_type,
                        source_id: ctx.source_id(),
                        span: ty.span,
                    });
                } else {
                    ctx.record_span(&zngur_type, ty.span.into_range());
                    checked_merge(
                        zngur_type,
                        &mut r.spec,
                        ty.span,
                        ctx,
                        Some(MergeContext {
                            outer_ty: rust_ty,
                            variant: None,
                        }),
                    );
                }
            }
            ProcessedItem::Trait { tr, methods } => {
                let trt = ZngurTrait {
                    tr: tr.inner.to_zngur(scope),
                    methods: methods.into_iter().map(|m| m.to_zngur(scope)).collect(),
                };
                ctx.record_span(&trt, tr.span.into_range());
                checked_merge(trt, &mut r.spec, tr.span, ctx, None);
            }
            ProcessedItem::Fn(f) => {
                let method = f.inner.to_zngur(scope);
                let func = ZngurFn {
                    path: RustPathAndGenerics {
                        path: scope.simple_relative_path(&method.name),
                        generics: method.generics,
                        named_generics: vec![],
                    },
                    inputs: method.inputs,
                    output: method.output,
                };
                ctx.record_span(&func, f.span.into_range());
                checked_merge(func, &mut r.spec, f.span, ctx, None);
            }
            ProcessedItem::ExternCpp(items) => {
                for item in items {
                    match item {
                        ParsedExternCppItem::Function { is_safe, method } => {
                            let span = method.span;
                            let method = method.inner.to_zngur(scope);
                            checked_merge(
                                ZngurExternCppFn {
                                    name: method.name.to_string(),
                                    inputs: method.inputs,
                                    output: method.output,
                                    is_safe,
                                },
                                &mut r.spec,
                                span,
                                ctx,
                                None,
                            );
                        }
                        ParsedExternCppItem::Impl { tr, ty, methods } => {
                            checked_merge(
                                ZngurExternCppImpl {
                                    tr: tr.map(|x| x.to_zngur(scope)),
                                    ty: ty.inner.to_zngur(scope),
                                    methods: methods
                                        .into_iter()
                                        .map(|(is_safe, x)| {
                                            let mut m = x.to_zngur(scope);
                                            m.is_safe = is_safe;
                                            m
                                        })
                                        .collect(),
                                },
                                &mut r.spec,
                                ty.span,
                                ctx,
                                None,
                            );
                        }
                    }
                }
            }
            ProcessedItem::CppAdditionalInclude(s) => {
                match AdditionalIncludes(s.to_owned()).merge(&mut r.spec) {
                    Ok(()) => {}
                    Err(_) => {
                        unreachable!() // For now, additional includes can't have conflicts.
                    }
                }
            }
            ProcessedItem::ConvertPanicToException(span) => {
                if ctx.depth > 0 {
                    ctx.add_error_str(
                        "Using `#convert_panic_to_exception` in imported zngur files is not supported. This directive can only be used in the main zngur file.",
                        span,
                    );
                    return;
                }
                match ConvertPanicToException(true).merge(&mut r.spec) {
                    Ok(()) => {}
                    Err(_) => {
                        unreachable!() // For now, CPtE also can't have conflicts.
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedRustType<'a> {
    Primitive(PrimitiveRustType),
    Ref(Mutability, Box<ParsedRustType<'a>>),
    Raw(Mutability, Box<ParsedRustType<'a>>),
    Boxed(Box<ParsedRustType<'a>>),
    Slice(Box<ParsedRustType<'a>>),
    Dyn(ParsedRustTrait<'a>, Vec<&'a str>),
    Impl(ParsedRustTrait<'a>, Vec<&'a str>),
    Tuple(Vec<ParsedRustType<'a>>),
    Adt(ParsedRustPathAndGenerics<'a>),
}

impl ParsedRustType<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> RustType {
        match self {
            ParsedRustType::Primitive(s) => RustType::Primitive(s),
            ParsedRustType::Ref(m, s) => RustType::Ref(m, Box::new(s.to_zngur(scope))),
            ParsedRustType::Raw(m, s) => RustType::Raw(m, Box::new(s.to_zngur(scope))),
            ParsedRustType::Boxed(s) => RustType::Boxed(Box::new(s.to_zngur(scope))),
            ParsedRustType::Slice(s) => RustType::Slice(Box::new(s.to_zngur(scope))),
            ParsedRustType::Dyn(tr, bounds) => RustType::Dyn(
                tr.to_zngur(scope),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Impl(tr, bounds) => RustType::Impl(
                tr.to_zngur(scope),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Tuple(v) => {
                RustType::Tuple(v.into_iter().map(|s| s.to_zngur(scope)).collect())
            }
            ParsedRustType::Adt(s) => match scope.as_type_var(&s) {
                Some(v) => RustType::TypeVar(v),
                None => RustType::Adt(s.to_zngur(scope)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedRustTrait<'a> {
    Normal(ParsedRustPathAndGenerics<'a>),
    Fn {
        name: &'a str,
        inputs: Vec<ParsedRustType<'a>>,
        output: Box<ParsedRustType<'a>>,
    },
}

impl ParsedRustTrait<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> RustTrait {
        match self {
            ParsedRustTrait::Normal(s) => RustTrait::Normal(s.to_zngur(scope)),
            ParsedRustTrait::Fn {
                name,
                inputs,
                output,
            } => RustTrait::Fn {
                name: name.to_owned(),
                inputs: inputs.into_iter().map(|s| s.to_zngur(scope)).collect(),
                output: Box::new(output.to_zngur(scope)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRustPathAndGenerics<'a> {
    path: ParsedPath<'a>,
    generics: Vec<ParsedRustType<'a>>,
    named_generics: Vec<(&'a str, ParsedRustType<'a>)>,
}

impl ParsedRustPathAndGenerics<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> RustPathAndGenerics {
        RustPathAndGenerics {
            path: scope.resolve_path(self.path),
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope))
                .collect(),
            named_generics: self
                .named_generics
                .into_iter()
                .map(|(name, x)| (name.to_owned(), x.to_zngur(scope)))
                .collect(),
        }
    }
}

pub type SourceId = usize;
pub type ReportSpan = (SourceId, std::ops::Range<usize>);
pub type ParseReport<'b> = Report<'b, ReportSpan>;

/// A diagnostic report, tagged with whether it should abort the parse.
pub struct ReportEntry<'b> {
    pub fatal: bool,
    pub report: ParseReport<'b>,
}

/// One half of a split key to identify a parsed item type
/// to store a list of spans for that type. The other half is a [`SourceId`](SourceId).
///
/// Uniquely identifying an individual span would require generating
/// and storing a declaration id in the parser and most spans are only
/// needed to identify conflicts when merging declarations so identifying
/// a declaration type is enough
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PartialSpanKey {
    /// an [`Import`](zngur_def::Import)
    Import(Box<std::path::Path>),
    /// a [`Type`](zngur_def::ZngurType)
    Ty(RustType),
    /// a [`Trait`](zngur_def::ZngurTrait)
    Trait(RustTrait),
    /// a [`Fn`](zngur_def::ZngurFn)
    Fn(ZngurFn),

    // spans likely to be inside others
    /// a [`LayoutPolicy`](zngur_def::LayoutPolicy)
    /// outer_ty
    Layout(RustType),
    /// a [`Constructor`](zngur_def::ZngurConstructor)
    /// outer_ty, constructor_sig?
    Constructor(RustType, Option<Vec<(String, RustType)>>),
    /// a [`CppRef`](zngur_def::CppRef)
    /// outer_ty
    CppRef(RustType),
    /// a [`CppHeapAllocated`](zngur_def::CppHeapAllocated)
    /// outer_ty
    CppHeapAllocated(RustType),
    /// a [`CppStackOwned`](zngur_def::CppStackOwned)
    /// outer_ty
    CppStackOwned(RustType),
    /// a [`Method`](zngur_def::ZngurMethod) on a
    /// [`Type`](zngur_def::ZngurType) or [`Trait`](zngur_def::ZngurTrait)
    /// outer_ty, method
    Method(RustType, ZngurMethod),
    /// a [`Field`](`zngur_def::ZngurField`) on a [`Type`](zngur_def::ZngurType)
    /// or it's internal [`Variant`](zngur_def::ZngurVariant)
    /// outer_ty, variant?, field_name
    Field(RustType, Option<String>, String),
}

impl PartialSpanKey {
    /// combine with a [`SourceId`] to make a full [`SpanKey`]
    fn full_key_with(self, source_id: SourceId) -> SpanKey {
        SpanKey(source_id, self)
    }
}

/// a full key identifying a list of spans for a given type in a particular source
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpanKey(SourceId, PartialSpanKey);

/// trait to impl for types to allow easy span lookup
trait IntoSpanKey {
    fn into_span_key(&self) -> PartialSpanKey;
}

impl IntoSpanKey for SpanKey {
    fn into_span_key(&self) -> PartialSpanKey {
        self.1.clone()
    }
}

impl IntoSpanKey for PartialSpanKey {
    fn into_span_key(&self) -> PartialSpanKey {
        self.clone()
    }
}

impl IntoSpanKey for zngur_def::Import {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Import(self.0.as_path().into())
    }
}

impl IntoSpanKey for zngur_def::ZngurType {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Ty(self.ty.clone())
    }
}
impl IntoSpanKey for zngur_def::RustType {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Ty(self.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurTrait {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Trait(self.tr.clone())
    }
}
impl IntoSpanKey for zngur_def::RustTrait {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Trait(self.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurFn {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Fn(self.clone())
    }
}

/// trait to impl for type that require variant and type qualifications for lookup
trait VariantQualifiedSpanKeyExt {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey;
}

/// trait to impl for type that require type qualifications for lookup
trait QualifiedSpanKeyExt {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey;
}

impl VariantQualifiedSpanKeyExt for zngur_def::ConflictSource {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey {
        match self {
            zngur_def::ConflictSource::Layout => PartialSpanKey::Layout(outer_ty),
            zngur_def::ConflictSource::Constructor(sig) => {
                PartialSpanKey::Constructor(outer_ty, sig.clone())
            }
            zngur_def::ConflictSource::CppRef => PartialSpanKey::CppRef(outer_ty),
            zngur_def::ConflictSource::CppHeapAllocated => {
                PartialSpanKey::CppHeapAllocated(outer_ty)
            }
            zngur_def::ConflictSource::CppStackOwned => PartialSpanKey::CppStackOwned(outer_ty),
            zngur_def::ConflictSource::Method(method) => {
                PartialSpanKey::Method(outer_ty, method.clone())
            }
            zngur_def::ConflictSource::Field(name) => {
                PartialSpanKey::Field(outer_ty, variant, name.clone())
            }
        }
    }
}

impl VariantQualifiedSpanKeyExt for zngur_def::ZngurField {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey {
        PartialSpanKey::Field(outer_ty, variant, self.name.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::LayoutPolicy {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Layout(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurMethod {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Method(outer_ty, self.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppHeapAllocated {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::CppHeapAllocated(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppRef {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::CppRef(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppStackOwned {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::CppStackOwned(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurConstructor {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Constructor(outer_ty, Some(self.inputs.clone()))
    }
}

/// A wrapper around both an owned value and a exclusive mutable
/// reference to that same value. Used to allow nested parse contexts
/// to internally borrow from their parent.
///
/// Implements the important [`AsMut`](AsMut) and
/// [`Deref`](Deref)/[`DerefMut`](DerefMut) traits to allow the type
/// to be used transparently.
enum OwnedRefMut<'a, T> {
    Owned(T),
    Borrowed(&'a mut T),
}

impl<'a, T> Deref for OwnedRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t,
        }
    }
}

impl<'a, T> DerefMut for OwnedRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t,
        }
    }
}

impl<'a, T, U> AsMut<U> for OwnedRefMut<'a, T>
where
    <OwnedRefMut<'a, T> as Deref>::Target: AsMut<U>,
{
    fn as_mut(&mut self) -> &mut U {
        self.deref_mut().as_mut()
    }
}

impl<'a, T, U> AsRef<U> for OwnedRefMut<'a, T>
where
    <OwnedRefMut<'a, T> as Deref>::Target: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        self.deref().as_ref()
    }
}

impl<'a, T> OwnedRefMut<'a, T> {
    /// exclusively borrows from self returning a wrapped value
    pub fn into_borrowed<'t>(&'t mut self) -> OwnedRefMut<'t, T> {
        match self {
            Self::Owned(t) => OwnedRefMut::<'t, T>::Borrowed(t),
            Self::Borrowed(t) => OwnedRefMut::<'t, T>::Borrowed(t),
        }
    }

    /// consume self to return the inner value if owned
    #[allow(dead_code)]
    pub fn into_inner(self) -> Option<T> {
        match self {
            Self::Owned(t) => Some(t),
            Self::Borrowed(_) => None,
        }
    }
}

impl<'a, T: Default> Default for OwnedRefMut<'a, T> {
    fn default() -> Self {
        Self::Owned(Default::default())
    }
}

impl<'a, T: Clone> OwnedRefMut<'a, T> {
    /// clones the inner value unconditionally
    pub fn clone_inner(&self) -> T {
        match self {
            Self::Owned(t) => t.clone(),
            Self::Borrowed(t) => (*t).clone(),
        }
    }

    /// returns the inner value, cloning a borrowed value if necessary
    #[allow(dead_code)]
    pub fn unwrap_or_clone(self) -> T {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct ReportCounter {
    pub errors: usize,
    pub warnings: usize,
}

/// The parse context. Holds references to the current source path and text and tracks
/// errors and declaration spans. Also holds the [configuration provider](RustCfgProvider)
/// and [report sink](ReportSink) used for this parse.
struct ParseContext<'this, 'source, 'cfg> {
    path: &'source std::path::Path,
    source: &'source str,
    source_id: SourceId,
    depth: usize,
    cfg_provider: &'cfg dyn RustCfgProvider,
    report_sink: &'cfg mut dyn ReportSink,
    /// All .zng files processed during parsing (main file + imports)
    processed_files: OwnedRefMut<'this, Vec<std::path::PathBuf>>,
    report_counter: OwnedRefMut<'this, ReportCounter>,
    sources: OwnedRefMut<'this, indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>>,
    recorded_spans: OwnedRefMut<'this, indexmap::IndexMap<SpanKey, Vec<ReportSpan>>>,
}

impl<'this, 'source, 'cfg> ParseContext<'this, 'source, 'cfg> {
    fn new(
        path: &'source std::path::Path,
        source: &'source str,
        cfg: &'cfg dyn RustCfgProvider,
        report_sink: &'cfg mut dyn ReportSink,
    ) -> Self {
        let processed_files = OwnedRefMut::Owned(vec![path.to_path_buf()]);
        let mut sources = OwnedRefMut::Owned(indexmap::IndexMap::default());
        let (source_id, _) = sources.insert_full(
            path.to_path_buf(),
            ariadne::Source::from(source.to_string()),
        );
        Self {
            path,
            source_id,
            source,
            depth: 0,
            cfg_provider: cfg,
            report_sink,
            processed_files,
            report_counter: Default::default(),
            sources,
            recorded_spans: Default::default(),
        }
    }

    /// build a nested parse context for parsing a new source
    /// to be merged into the current one
    fn nested<'borrowed, 'src>(
        &'borrowed mut self,
        path: &'src std::path::Path,
        source: &'src str,
    ) -> ParseContext<'borrowed, 'src, 'borrowed> {
        let (source_id, _) = self.sources.insert_full(
            path.to_path_buf(),
            ariadne::Source::from(source.to_string()),
        );
        self.processed_files.push(path.to_path_buf());
        ParseContext {
            path,
            source_id,
            source,
            depth: self.depth + 1,
            cfg_provider: self.cfg_provider,
            report_sink: self.report_sink,
            processed_files: self.processed_files.into_borrowed(),
            report_counter: self.report_counter.into_borrowed(),
            sources: self.sources.into_borrowed(),
            recorded_spans: self.recorded_spans.into_borrowed(),
        }
    }

    /// the [`SourceId`] of the current source which can be fetched from the cache
    /// retrieved by calling [`source_cache`](Self::source_cache)
    fn source_id(&self) -> SourceId {
        self.source_id
    }

    /// returns a borrowed source cache that implements the ariadne Cache trait
    #[allow(dead_code)]
    fn source_cache(&self) -> SourceCache<'_> {
        SourceCache::new(&self.sources)
    }

    /// Renders a single report to text (stripped of ANSI color codes).
    #[allow(dead_code)]
    fn render_report(&self, report: &ParseReport) -> String {
        let mut buf = Vec::new();
        report.write(self.source_cache(), &mut buf).unwrap();
        String::from_utf8(strip_ansi_escapes::strip(buf)).unwrap()
    }

    /// Record a report that should invalidate the current parse
    fn add_fatal_report<'r>(&mut self, report: ParseReport<'r>) {
        self.sink_report(ReportEntry {
            fatal: true,
            report,
        });
    }

    /// Record a non-fatal report
    fn add_report<'r>(&mut self, report: ParseReport<'r>) {
        self.sink_report(ReportEntry {
            fatal: false,
            report,
        });
    }

    /// Record fatal reports from the passed parse errors
    fn add_errors<'err_src>(&mut self, errs: impl Iterator<Item = Rich<'err_src, String>>) {
        let path = self.source_id;
        for e in errs {
            let e_span = (path.to_owned(), e.span().into_range());
            let report = Report::build(ReportKind::Error, e_span.clone())
                .with_message(e.to_string())
                .with_label(
                    Label::new(e_span)
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    let l_span = (path.to_owned(), span.into_range());
                    Label::new(l_span)
                        .with_message(format!("while parsing this {}", label))
                        .with_color(Color::Yellow)
                }))
                .finish();
            self.sink_report(ReportEntry {
                fatal: true,
                report,
            });
        }
    }

    /// Record a fatal report from the passed message and span
    fn add_error_str(&mut self, error: &str, span: Span) {
        self.add_errors([Rich::custom(span, error)].into_iter());
    }

    /// Records a non-fatal warning as an ariadne diagnostic pointing at `span`.
    /// Rendering is deferred until dispatch time (see `render_report`).
    fn add_warning_str(&mut self, warning: &str, span: Span) {
        let report = Report::build(
            ReportKind::Warning,
            (self.source_id.to_owned(), span.into_range()),
        )
        .with_message(warning)
        .with_label(
            Label::new((self.source_id.to_owned(), span.into_range()))
                .with_message(warning)
                .with_color(Color::Yellow),
        )
        .finish();
        self.sink_report(ReportEntry {
            fatal: false,
            report,
        });
    }

    /// a count of the errors recorded by this parse context and any nested ones
    fn reports(&self) -> &ReportCounter {
        &self.report_counter
    }

    /// drains and sinks the current reports
    /// returns true if any errors were reported
    fn sink_report<'r>(&mut self, report: ReportEntry<'r>) {
        let ParseContext {
            report_sink,
            sources,
            report_counter,
            ..
        } = self;
        let report_counter = report_counter.deref_mut();
        if report.fatal {
            report_counter.errors += 1;
        } else {
            report_counter.warnings += 1;
        }
        let source_cache = SourceCache::new(sources);
        report_sink.sink_report(&report, source_cache);
    }

    fn get_config_provider(&self) -> &dyn RustCfgProvider {
        self.cfg_provider
    }

    fn get_processed_files(&self) -> Vec<std::path::PathBuf> {
        self.processed_files.clone_inner()
    }

    /// Record a span for the key as existing in the currently processing file
    fn record_span<K: IntoSpanKey>(&mut self, key: &K, range: std::ops::Range<usize>) {
        let span = self.report_span_for_range(range);
        use indexmap::map::Entry;
        match self
            .recorded_spans
            .entry(key.into_span_key().full_key_with(self.source_id))
        {
            Entry::Occupied(mut spans) => {
                let spans = spans.get_mut();
                if !spans.contains(&span) {
                    spans.push(span);
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![span]);
            }
        }
    }

    /// Transform a range into a [report span](ReportSpan) for the currently processing file
    fn report_span_for_range(&self, range: std::ops::Range<usize>) -> ReportSpan {
        (self.source_id.to_owned(), range)
    }

    /// fetch the recorded spans for the item in the currently processing source
    /// stored in order of declaration
    fn fetch_spans_current<K: IntoSpanKey>(&self, key: &K) -> Vec<&ReportSpan> {
        self.fetch_spans_for(key, self.source_id)
    }

    /// fetch the recorded spans for the item in the given source
    /// stored in order of declaration
    fn fetch_spans_for<K: IntoSpanKey>(&self, key: &K, source_id: SourceId) -> Vec<&ReportSpan> {
        self.recorded_spans
            .get(&key.into_span_key().full_key_with(source_id))
            .into_iter()
            .flatten()
            .collect()
    }

    /// fetch the recorded spans for the item in all sources
    /// stored in order of declaration
    fn fetch_spans_global<K: IntoSpanKey>(&self, key: &K) -> Vec<&ReportSpan> {
        let partial = key.into_span_key();
        self.recorded_spans
            .iter()
            .filter_map(
                |(SpanKey(_, part), spans)| {
                    if *part == partial { Some(spans) } else { None }
                },
            )
            .flatten()
            .collect()
    }
}

/// A trait for types which can resolve filesystem-like paths relative to a given directory.
pub trait ImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String>;
}

/// A default implementation of ImportResolver which uses conventional filesystem paths and semantics.
pub struct DefaultImportResolver;

impl ImportResolver for DefaultImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd
            .join(relpath)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}

impl<T: ImportResolver + ?Sized> ImportResolver for Box<T> {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        self.deref().resolve_import(cwd, relpath)
    }
}

/// a [`ariadne::Cache<SourceId>`] compatible source cache
/// internally maps a [`SourceId`] to a [`ariadne::Source<String>`]
/// borrowed from the [`ParseContext`]
#[derive(Debug, Clone, Copy)]
pub struct SourceCache<'c> {
    sources: &'c indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>,
}

impl<'c> SourceCache<'c> {
    fn new(sources: &'c indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>) -> Self {
        Self { sources }
    }
}

impl<'c> ariadne::Cache<SourceId> for SourceCache<'c> {
    type Storage = String;
    fn fetch(
        &mut self,
        id: &SourceId,
    ) -> Result<&ariadne::Source<Self::Storage>, impl std::fmt::Debug> {
        match self.sources.get_index(*id) {
            Some((_path, source)) => Ok(source),
            None => Err(format!("Unknown source id '{}'", id)),
        }
    }

    fn display<'a>(&self, id: &'a SourceId) -> Option<impl std::fmt::Display + 'a> {
        self.sources
            .get_index(*id)
            .map(|(path, _source)| path.display().to_string())
    }
}

/// a type that can receive reports from the zngur parser as they are generated
pub trait ReportSink {
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache);
}

impl<T> ReportSink for T
where
    T: for<'r, 'c> FnMut(bool, &ParseReport<'r>, SourceCache<'c>),
{
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache) {
        self(report.fatal, &report.report, source_cache)
    }
}

/// a [`ReportSink`] that prints to `stderr`
pub struct StdErrReportSink<const ERRORS: bool = true, const WARNINGS: bool = true>;

impl<const ERRORS: bool, const WARNINGS: bool> ReportSink for StdErrReportSink<ERRORS, WARNINGS> {
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache) {
        if (report.fatal && ERRORS) || (!report.fatal && WARNINGS) {
            let _ = report.report.eprint(source_cache);
        }
    }
}

impl<'a> ParsedZngFile<'a> {
    fn parse_into(
        zngur: &mut ZngurSpecBuilder,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let (tokens, errs) = lexer().parse(ctx.source).into_output_errors();
        let Some(tokens) = tokens else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            return;
        };
        let tokens: ParserInput<'_> = tokens.as_slice().map(
            (ctx.source.len()..ctx.source.len()).into(),
            Box::new(|(t, s)| (t, s)),
        );
        let (ast, errs) = file_parser()
            .map_with(|ast, extra| (ast, extra.span()))
            .parse_with_state(tokens, &mut extra::SimpleState(ZngParserState::default()))
            .into_output_errors();
        let Some(ast) = ast else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            return;
        };

        let (aliases, items) = partition_parsed_items(
            ast.0
                .0
                .into_iter()
                .map(|item| process_parsed_item(item, ctx)),
        );
        ProcessedZngFile::new(aliases, items).into_zngur_spec(zngur, ctx);
        if ctx.reports().errors > 0 {
            return;
        }

        if let Some(dirname) = ctx.path.to_owned().parent() {
            for import in std::mem::take(&mut zngur.imports) {
                match resolver.resolve_import(dirname, &import.0) {
                    Ok(text) => {
                        let path = dirname.join(&import.0);
                        let mut nested_ctx = ctx.nested(&path, &text);
                        Self::parse_into(zngur, &mut nested_ctx, resolver);
                    }
                    Err(err) => {
                        let path = import.0.display();
                        let spans = ctx.fetch_spans_current(&import);
                        let (first, rest) = {
                            let mut iter = spans.into_iter();
                            let first = iter
                                .next()
                                .cloned()
                                .unwrap_or_else(|| ctx.report_span_for_range(0..0));
                            let rest: Vec<_> = iter.collect();
                            (first, rest)
                        };
                        let mut report = Report::build(ReportKind::Error, first)
                            .with_message(format!("Failed to process merge file `{path}`: {err}",));
                        let count = rest.len();
                        for (i, span) in rest.into_iter().enumerate() {
                            report = report.with_label(
                                ariadne::Label::new(span.clone())
                                    .with_message("Also merged here")
                                    .with_color(Color::Blue),
                            );
                            if i >= 2 {
                                report = report.with_note(format!(
                                    "{} additional locations omitted",
                                    count - i
                                ));
                                break;
                            }
                        }

                        ctx.add_fatal_report(report.finish());
                    }
                }
            }
        }
    }

    /// Parse a .zng file and return both the spec and list of all processed files.
    ///
    /// `warning_sink` is called once per deprecation or other non-fatal warning
    /// produced while parsing.
    pub fn parse(
        path: &std::path::Path,
        cfg: impl RustCfgProvider + 'static,
        report_sink: &mut impl ReportSink,
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        zngur.spec.rust_cfg.extend(cfg.get_cfg_pairs());
        zngur.spec.rust_cfg.sort();
        let text = std::fs::read_to_string(&path).unwrap();
        let cfg: &dyn RustCfgProvider = &cfg;
        let mut ctx = ParseContext::new(path, &text, cfg, report_sink);
        Self::parse_into(&mut zngur, &mut ctx, &DefaultImportResolver);
        let spec = zngur.to_zngur(&mut ctx);
        if ctx.reports().errors > 0 {
            // add report of cfg values used
            ctx.add_report(
                Report::build(
                    ReportKind::Custom("cfg values used", ariadne::Color::Green),
                    ctx.report_span_for_range(0..0),
                )
                .with_message(
                    cfg.get_cfg_pairs()
                        .into_iter()
                        .map(|(key, value)| match value {
                            Some(value) => format!("{key}=\"{value}\""),
                            None => key,
                        })
                        .join("\n")
                        .to_string(),
                )
                .finish(),
            );
        }
        ParseResult {
            spec,
            processed_files: ctx.get_processed_files(),
            errors: ctx.reports().errors,
            warnings: ctx.reports().warnings,
        }
    }

    /// parse a IDL source str with with given options.
    /// sinks errors and warnings instead of panicking
    pub fn parse_str_with_resolver(
        text: &str,
        path: &str,
        cfg: impl RustCfgProvider + 'static,
        resolver: &impl ImportResolver,
        report_sink: &mut impl ReportSink,
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        let path = std::path::PathBuf::from(path);
        let mut ctx = ParseContext::new(&path, text, &cfg, report_sink);
        Self::parse_into(&mut zngur, &mut ctx, resolver);
        let spec = zngur.to_zngur(&mut ctx);
        ParseResult {
            spec,
            processed_files: ctx.get_processed_files(),
            errors: ctx.reports().errors,
            warnings: ctx.reports().warnings,
        }
    }
}

pub(crate) enum ProcessedItemOrAlias<'a> {
    Ignore,
    Processed(ProcessedItem<'a>),
    Alias(ParsedAlias<'a>),
    ChildItems(Vec<ProcessedItemOrAlias<'a>>),
}

fn process_parsed_item<'a>(
    item: ParsedItem<'a>,
    ctx: &mut ParseContext,
) -> ProcessedItemOrAlias<'a> {
    use ProcessedItemOrAlias as Ret;
    match item {
        ParsedItem::Alias(alias) => Ret::Alias(alias),
        ParsedItem::ConvertPanicToException(span) => {
            Ret::Processed(ProcessedItem::ConvertPanicToException(span))
        }
        ParsedItem::UnstableFeature(_) => {
            // ignore
            Ret::Ignore
        }
        ParsedItem::CppAdditionalInclude(inc) => {
            Ret::Processed(ProcessedItem::CppAdditionalInclude(inc))
        }
        ParsedItem::Mod { path, items } => {
            let (aliases, items) = partition_parsed_items(
                items.into_iter().map(|item| process_parsed_item(item, ctx)),
            );
            Ret::Processed(ProcessedItem::Mod {
                path,
                items,
                aliases,
            })
        }
        ParsedItem::Type {
            ty,
            items,
            type_vars,
        } => Ret::Processed(ProcessedItem::Type {
            ty,
            items,
            type_vars,
        }),
        ParsedItem::Trait { tr, methods } => Ret::Processed(ProcessedItem::Trait { tr, methods }),
        ParsedItem::Fn(method) => Ret::Processed(ProcessedItem::Fn(method)),
        ParsedItem::ExternCpp(items) => Ret::Processed(ProcessedItem::ExternCpp(items)),
        ParsedItem::Import(path) => Ret::Processed(ProcessedItem::Import(path)),
        ParsedItem::ModuleImport { path, span } => {
            Ret::Processed(ProcessedItem::ModuleImport { path, span })
        }
        ParsedItem::MatchOnCfg(match_) => Ret::ChildItems(
            match_
                .eval(ctx)
                .unwrap_or_default() // unwrap or empty
                .into_iter()
                .map(|item| item.inner)
                .collect(),
        ),
    }
}

fn partition_parsed_items<'a>(
    items: impl IntoIterator<Item = ProcessedItemOrAlias<'a>>,
) -> (Vec<ParsedAlias<'a>>, Vec<ProcessedItem<'a>>) {
    let mut aliases = Vec::new();
    let mut processed = Vec::new();
    for item in items.into_iter() {
        match item {
            ProcessedItemOrAlias::Ignore => continue,
            ProcessedItemOrAlias::Processed(p) => processed.push(p),
            ProcessedItemOrAlias::Alias(a) => aliases.push(a),
            ProcessedItemOrAlias::ChildItems(children) => {
                let (child_aliases, child_items) = partition_parsed_items(children);
                aliases.extend(child_aliases);
                processed.extend(child_items);
            }
        }
    }
    (aliases, processed)
}

impl<'a> ProcessedZngFile<'a> {
    fn new(aliases: Vec<ParsedAlias<'a>>, items: Vec<ProcessedItem<'a>>) -> Self {
        ProcessedZngFile { aliases, items }
    }

    fn into_zngur_spec(self, zngur: &mut ZngurSpecBuilder, ctx: &mut ParseContext) {
        let root_scope = Scope::new_root(self.aliases);

        for item in self.items {
            item.add_to_zngur_spec(zngur, &root_scope, ctx);
        }
    }
}

struct TemplateDef {
    ty: ZngurType,
    source_id: SourceId,
    span: Span,
}

#[derive(Default)]
struct ZngurSpecBuilder {
    spec: ZngurSpec,
    templates: Vec<TemplateDef>,
    imports: Vec<Import>,
}

impl ZngurSpecBuilder {
    fn to_zngur(self, ctx: &mut ParseContext) -> ZngurSpec {
        let ZngurSpecBuilder {
            mut spec,
            templates,
            imports: _,
        } = self;
        for ty in &mut spec.types {
            let mut template_locations = Vec::new();
            for template in &templates {
                if let Some(template_match) = try_match_template(&ty.ty, &template.ty) {
                    let location = (template.source_id, template.span.into_range());
                    if let Err(e) = template_match.merge(ty) {
                        match e {
                            MergeFailure::Conflict(msg, conflict) => {
                                let report = build_template_conflict_report(
                                    ctx,
                                    template,
                                    &ty,
                                    location.clone(),
                                    &msg,
                                    conflict,
                                );

                                ctx.add_fatal_report(report);
                            }
                        }
                    } else {
                        template_locations.push(location);
                    }
                }
            }
            if !ty.wellknown_traits.iter().any(|wkt| {
                matches!(
                    wkt,
                    ZngurWellknownTrait::Copy | ZngurWellknownTrait::Unsized
                )
            }) {
                ty.wellknown_traits.push(ZngurWellknownTrait::Drop);
            }
            if ty.layout.is_none() {
                let spans = ctx.fetch_spans_global(ty);
                let (first, rest) = {
                    let mut it = spans.into_iter();
                    let first = it.next().cloned();
                    (first, it.collect::<Vec<_>>())
                };
                let mut report = Report::build(ReportKind::Error, (0, 0usize..0)).with_message(format!(
                    "No layout policy found for type {}.",
                    ty.ty
                )).with_note("Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.");

                if let Some(first) = first {
                    report.add_label(
                        Label::new(first)
                            .with_message("Type first declared here.")
                            .with_color(Color::Blue),
                    );
                }
                let count = rest.len();
                for (i, span) in rest.into_iter().enumerate() {
                    report = report.with_label(
                        Label::new(span.clone())
                            .with_message("Type also declared here")
                            .with_color(Color::Blue),
                    );
                    if i >= 2 {
                        report = report.with_note(format!(
                            "{} additional type declaration locations omitted",
                            count - i
                        ));
                        break;
                    }
                }
                let t_count = template_locations.len();
                for (i, location) in template_locations.into_iter().enumerate() {
                    report = report.with_label(
                        Label::new(location)
                            .with_message("Matching template defined here")
                            .with_color(Color::Blue),
                    );
                    if i >= 2 {
                        report = report.with_note(format!(
                            "{} additional template declaration locations omitted",
                            t_count - i
                        ));
                        break;
                    }
                }
                ctx.add_fatal_report(report.finish());
            }
        }
        spec
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Token<'a> {
    Arrow,
    ArrowArm,
    AngleOpen,
    AngleClose,
    BracketOpen,
    BracketClose,
    Colon,
    ColonColon,
    ParenOpen,
    ParenClose,
    BraceOpen,
    BraceClose,
    And,
    Star,
    Sharp,
    Plus,
    Eq,
    Question,
    Comma,
    Semicolon,
    Pipe,
    Underscore,
    Dot,
    Bang,
    KwAs,
    KwAsync,
    KwDyn,
    KwUse,
    KwFor,
    KwMod,
    KwCrate,
    KwType,
    KwTrait,
    KwFn,
    KwMut,
    KwConst,
    KwExtern,
    KwImpl,
    KwImport,
    KwMerge,
    KwIf,
    KwElse,
    KwMatch,
    KwSafe,
    KwUnsafe,
    Ident(&'a str),
    Str(&'a str),
    RawStr(usize, &'a str),
    Number(usize),
}

impl<'a> Token<'a> {
    fn ident_or_kw(ident: &'a str) -> Self {
        match ident {
            "as" => Token::KwAs,
            "async" => Token::KwAsync,
            "dyn" => Token::KwDyn,
            "mod" => Token::KwMod,
            "type" => Token::KwType,
            "trait" => Token::KwTrait,
            "crate" => Token::KwCrate,
            "fn" => Token::KwFn,
            "mut" => Token::KwMut,
            "const" => Token::KwConst,
            "use" => Token::KwUse,
            "for" => Token::KwFor,
            "extern" => Token::KwExtern,
            "impl" => Token::KwImpl,
            "import" => Token::KwImport,
            "merge" => Token::KwMerge,
            "if" => Token::KwIf,
            "else" => Token::KwElse,
            "match" => Token::KwMatch,
            "safe" => Token::KwSafe,
            "unsafe" => Token::KwUnsafe,
            x => Token::Ident(x),
        }
    }
}

impl Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Arrow => write!(f, "->"),
            Token::ArrowArm => write!(f, "=>"),
            Token::AngleOpen => write!(f, "<"),
            Token::AngleClose => write!(f, ">"),
            Token::BracketOpen => write!(f, "["),
            Token::BracketClose => write!(f, "]"),
            Token::ParenOpen => write!(f, "("),
            Token::ParenClose => write!(f, ")"),
            Token::BraceOpen => write!(f, "{{"),
            Token::BraceClose => write!(f, "}}"),
            Token::Colon => write!(f, ":"),
            Token::ColonColon => write!(f, "::"),
            Token::And => write!(f, "&"),
            Token::Star => write!(f, "*"),
            Token::Sharp => write!(f, "#"),
            Token::Plus => write!(f, "+"),
            Token::Eq => write!(f, "="),
            Token::Question => write!(f, "?"),
            Token::Comma => write!(f, ","),
            Token::Semicolon => write!(f, ";"),
            Token::Pipe => write!(f, "|"),
            Token::Underscore => write!(f, "_"),
            Token::Dot => write!(f, "."),
            Token::Bang => write!(f, "!"),
            Token::KwAs => write!(f, "as"),
            Token::KwAsync => write!(f, "async"),
            Token::KwDyn => write!(f, "dyn"),
            Token::KwUse => write!(f, "use"),
            Token::KwFor => write!(f, "for"),
            Token::KwMod => write!(f, "mod"),
            Token::KwCrate => write!(f, "crate"),
            Token::KwType => write!(f, "type"),
            Token::KwTrait => write!(f, "trait"),
            Token::KwFn => write!(f, "fn"),
            Token::KwMut => write!(f, "mut"),
            Token::KwConst => write!(f, "const"),
            Token::KwExtern => write!(f, "extern"),
            Token::KwImpl => write!(f, "impl"),
            Token::KwImport => write!(f, "import"),
            Token::KwMerge => write!(f, "merge"),
            Token::KwIf => write!(f, "if"),
            Token::KwElse => write!(f, "else"),
            Token::KwMatch => write!(f, "match"),
            Token::KwSafe => write!(f, "safe"),
            Token::KwUnsafe => write!(f, "unsafe"),
            Token::Ident(i) => write!(f, "{i}"),
            Token::Number(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, r#""{s}""#),
            Token::RawStr(hashes, s) => {
                let h = "#".repeat(*hashes);
                write!(f, r#"r{h}"{s}"{h}"#)
            }
        }
    }
}

fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<(Token<'src>, Span)>, extra::Err<Rich<'src, char, Span>>> {
    let plain_string = just('"')
        .ignore_then(none_of('"').repeated().to_slice().map(Token::Str))
        .then_ignore(just('"'));

    let raw_string_start = just('r')
        .ignore_then(just('#').repeated().count())
        .then_ignore(just('"'));
    let raw_string_end =
        just('"').then(just('#').repeated().configure(|cfg, ctx| cfg.exactly(*ctx)));
    let raw_string = raw_string_start
        .then_with_ctx(
            any()
                .and_is(raw_string_end.not())
                .repeated()
                .to_slice()
                .then_ignore(raw_string_end),
        )
        .map(|(h, s)| Token::RawStr(h, s));

    let token = choice((
        choice([
            just("->").to(Token::Arrow),
            just("=>").to(Token::ArrowArm),
            just("<").to(Token::AngleOpen),
            just(">").to(Token::AngleClose),
            just("[").to(Token::BracketOpen),
            just("]").to(Token::BracketClose),
            just("(").to(Token::ParenOpen),
            just(")").to(Token::ParenClose),
            just("{").to(Token::BraceOpen),
            just("}").to(Token::BraceClose),
            just("::").to(Token::ColonColon),
            just(":").to(Token::Colon),
            just("&").to(Token::And),
            just("*").to(Token::Star),
            just("#").to(Token::Sharp),
            just("+").to(Token::Plus),
            just("=").to(Token::Eq),
            just("?").to(Token::Question),
            just(",").to(Token::Comma),
            just(";").to(Token::Semicolon),
            just("|").to(Token::Pipe),
            just("_").to(Token::Underscore),
            just(".").to(Token::Dot),
            just("!").to(Token::Bang),
        ]),
        raw_string,
        plain_string,
        text::ident().map(Token::ident_or_kw),
        text::int(10).map(|x: &str| Token::Number(x.parse().unwrap())),
    ));

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    token
        .map_with(|tok, extra| (tok, extra.span()))
        .padded_by(comment.repeated())
        .padded()
        .repeated()
        .collect()
        .boxed()
}

fn alias<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::KwUse)
        .ignore_then(path())
        .then_ignore(just(Token::KwAs))
        .then(select! {
            Token::Ident(c) => c,
        })
        .then_ignore(just(Token::Semicolon))
        .map_with(|(path, name), extra| {
            ParsedItem::Alias(ParsedAlias {
                name,
                path,
                span: extra.span(),
            })
        })
        .boxed()
}

fn file_parser<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedZngFile<'a>, ZngParserExtra<'a>> + Clone {
    item()
        .repeated()
        .collect::<Vec<_>>()
        .map(ParsedZngFile)
        .boxed()
}

fn rust_type<'a>() -> Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>> {
    let as_scalar = |s: &str, head: char| -> Option<u32> {
        let s = s.strip_prefix(head)?;
        s.parse().ok()
    };

    let scalar = select! {
        Token::Ident("bool") => PrimitiveRustType::Bool,
        Token::Ident("str") => PrimitiveRustType::Str,
        Token::Ident("char") => PrimitiveRustType::Char,
        Token::Ident("usize") => PrimitiveRustType::Usize,
        Token::Ident(c) if as_scalar(c, 'u').is_some() => PrimitiveRustType::Uint(as_scalar(c, 'u').unwrap()),
        Token::Ident(c) if as_scalar(c, 'i').is_some() => PrimitiveRustType::Int(as_scalar(c, 'i').unwrap()),
        Token::Ident(c) if as_scalar(c, 'f').is_some() => PrimitiveRustType::Float(as_scalar(c, 'f').unwrap()),
    }.map(ParsedRustType::Primitive);

    recursive(|parser| {
        let parser = parser.boxed();
        let pg = rust_path_and_generics(parser.clone());
        let adt = pg.clone().map(ParsedRustType::Adt);

        let dyn_trait = just(Token::KwDyn)
            .or(just(Token::KwImpl))
            .then(rust_trait(parser.clone()))
            .then(
                just(Token::Plus)
                    .ignore_then(select! {
                        Token::Ident(c) => c,
                    })
                    .repeated()
                    .collect::<Vec<_>>()
                    .boxed(),
            )
            .map(|((token, first), rest)| match token {
                Token::KwDyn => ParsedRustType::Dyn(first, rest),
                Token::KwImpl => ParsedRustType::Impl(first, rest),
                _ => unreachable!(),
            });
        let boxed = just(Token::Ident("Box"))
            .then(rust_generics(parser.clone()))
            .map(|(_, x)| {
                assert_eq!(x.len(), 1);
                ParsedRustType::Boxed(Box::new(x.into_iter().next().unwrap().right().unwrap()))
            });
        let unit = just(Token::ParenOpen)
            .then(just(Token::ParenClose))
            .map(|_| ParsedRustType::Tuple(vec![]));
        let tuple = parser
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
            .map(|xs| ParsedRustType::Tuple(xs));
        let slice = parser
            .clone()
            .map(|x| ParsedRustType::Slice(Box::new(x)))
            .delimited_by(just(Token::BracketOpen), just(Token::BracketClose));
        let reference = just(Token::And)
            .ignore_then(
                just(Token::KwMut)
                    .to(Mutability::Mut)
                    .or(empty().to(Mutability::Not)),
            )
            .then(parser.clone())
            .map(|(m, x)| ParsedRustType::Ref(m, Box::new(x)));
        let raw_ptr = just(Token::Star)
            .ignore_then(
                just(Token::KwMut)
                    .to(Mutability::Mut)
                    .or(just(Token::KwConst).to(Mutability::Not)),
            )
            .then(parser)
            .map(|(m, x)| ParsedRustType::Raw(m, Box::new(x)));
        choice((
            scalar.boxed(),
            boxed.boxed(),
            unit.boxed(),
            tuple.boxed(),
            slice.boxed(),
            adt.boxed(),
            reference.boxed(),
            raw_ptr.boxed(),
            dyn_trait.boxed(),
        ))
    })
    .boxed()
}

fn rust_generics<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<
    'a,
    ParserInput<'a>,
    Vec<Either<(&'a str, ParsedRustType<'a>), ParsedRustType<'a>>>,
    ZngParserExtra<'a>,
> + Clone {
    let named_generic = select! {
        Token::Ident(c) => c,
    }
    .then_ignore(just(Token::Eq))
    .then(rust_type.clone())
    .map(Either::Left);
    just(Token::ColonColon)
        .repeated()
        .at_most(1)
        .ignore_then(
            named_generic
                .or(rust_type.clone().map(Either::Right))
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .boxed(),
        )
        .boxed()
}

fn rust_path_and_generics<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, ParsedRustPathAndGenerics<'a>, ZngParserExtra<'a>> + Clone {
    let generics = rust_generics(rust_type.clone());
    path()
        .then(generics.clone().repeated().at_most(1).collect::<Vec<_>>())
        .map(|x| {
            let generics = x.1.into_iter().next().unwrap_or_default();
            let (named_generics, generics) = generics.into_iter().partition_map(|x| x);
            ParsedRustPathAndGenerics {
                path: x.0,
                generics,
                named_generics,
            }
        })
        .boxed()
}

fn fn_args<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, (Vec<ParsedRustType<'a>>, ParsedRustType<'a>), ZngParserExtra<'a>>
+ Clone {
    rust_type
        .clone()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .then(
            just(Token::Arrow)
                .ignore_then(rust_type)
                .or(empty().to(ParsedRustType::Tuple(vec![]))),
        )
        .boxed()
}

fn spanned<'a, T>(
    parser: impl Parser<'a, ParserInput<'a>, T, ZngParserExtra<'a>> + Clone,
) -> impl Parser<'a, ParserInput<'a>, Spanned<T>, ZngParserExtra<'a>> + Clone {
    parser.map_with(|inner, extra| Spanned {
        inner,
        span: extra.span(),
    })
}

fn rust_trait<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, ParsedRustTrait<'a>, ZngParserExtra<'a>> + Clone {
    let fn_trait = select! {
        Token::Ident(c) => c,
    }
    .then(fn_args(rust_type.clone()))
    .map(|x| ParsedRustTrait::Fn {
        name: x.0,
        inputs: x.1.0,
        output: Box::new(x.1.1),
    });

    let rust_trait = fn_trait.or(rust_path_and_generics(rust_type).map(ParsedRustTrait::Normal));
    rust_trait.boxed()
}

fn method<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedMethod<'a>, ZngParserExtra<'a>> + Clone {
    spanned(just(Token::KwAsync))
        .or_not()
        .then_ignore(just(Token::KwFn))
        .then(select! {
            Token::Ident(c) => c,
        })
        .then(
            rust_type()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .or(empty().to(vec![]))
                .boxed(),
        )
        .then(fn_args(rust_type()))
        .map(|(((opt_async, name), generics), args)| {
            let is_self = |c: &ParsedRustType<'_>| {
                if let ParsedRustType::Adt(c) = c {
                    c.path.start == ParsedPathStart::Relative
                        && &c.path.segments == &["self"]
                        && c.generics.is_empty()
                } else {
                    false
                }
            };
            let (inputs, receiver) = match args.0.get(0) {
                Some(x) if is_self(&x) => (args.0[1..].to_vec(), ZngurMethodReceiver::Move),
                Some(ParsedRustType::Ref(m, x)) if is_self(&x) => {
                    (args.0[1..].to_vec(), ZngurMethodReceiver::Ref(*m))
                }
                _ => (args.0, ZngurMethodReceiver::Static),
            };
            let mut output = args.1;
            if let Some(async_kw) = opt_async {
                output = ParsedRustType::Impl(
                    ParsedRustTrait::Normal(ParsedRustPathAndGenerics {
                        path: ParsedPath {
                            start: ParsedPathStart::Absolute,
                            segments: vec!["std", "future", "Future"],
                            span: async_kw.span,
                        },
                        generics: vec![],
                        named_generics: vec![("Output", output)],
                    }),
                    vec![],
                )
            }
            ParsedMethod {
                name,
                receiver,
                generics,
                inputs,
                output,
            }
        })
        .boxed()
}

fn inner_type_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedTypeItem<'a>, ZngParserExtra<'a>> + Clone {
    let property_item = (spanned(select! {
        Token::Ident(c) => c,
    }))
    .then_ignore(just(Token::Eq))
    .then(select! {
        Token::Number(c) => c,
    });
    let layout = just([Token::Sharp, Token::Ident("layout")])
        .ignore_then(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(ParsedLayoutPolicy::StackAllocated)
        .or(just([Token::Sharp, Token::Ident("layout_conservative")])
            .ignore_then(
                property_item
                    .clone()
                    .separated_by(just(Token::Comma))
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                    .boxed(),
            )
            .map(ParsedLayoutPolicy::Conservative))
        .or(just([Token::Sharp, Token::Ident("only_by_ref")]).to(ParsedLayoutPolicy::OnlyByRef))
        .or(just([Token::Sharp, Token::Ident("heap_allocated")])
            .to(ParsedLayoutPolicy::HeapAllocated))
        .map_with(|x, extra| ParsedTypeItem::Layout(extra.span(), x))
        .boxed();
    let trait_item = select! {
        Token::Ident("Debug") => ZngurWellknownTrait::Debug,
        Token::Ident("Copy") => ZngurWellknownTrait::Copy,
    }
    .or(just(Token::Question)
        .then(just(Token::Ident("Sized")))
        .to(ZngurWellknownTrait::Unsized));
    let traits = just(Token::Ident("wellknown_traits"))
        .ignore_then(
            spanned(trait_item)
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(ParsedTypeItem::Traits)
        .boxed();
    let non_exhaustive =
        just(Token::Ident("non_exhaustive")).map(|_| ParsedTypeItem::NonExhaustive);
    let constructor_args = rust_type()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .map(ParsedConstructorArgs::Tuple)
        .or((select! {
            Token::Ident(c) => c,
        })
        .boxed()
        .then_ignore(just(Token::Colon))
        .then(rust_type())
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
        .map(ParsedConstructorArgs::Named))
        .or(empty().to(ParsedConstructorArgs::Unit))
        .boxed();
    let constructor = just(Token::Ident("constructor"))
        .ignore_then(constructor_args)
        .map(|args| ParsedTypeItem::Constructor { args });
    let field = just(Token::Ident("field")).ignore_then(
        (select! {
            Token::Ident(c) => c.to_owned(),
            Token::Number(c) => c.to_string(),
        })
        .then(
            just(Token::Ident("offset"))
                .then(just(Token::Eq))
                .ignore_then(select! {
                    Token::Number(c) => Some(c),
                    Token::Ident("auto") => None,
                })
                .then(
                    just(Token::Comma)
                        .then(just(Token::KwType))
                        .then(just(Token::Eq))
                        .ignore_then(rust_type()),
                )
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(|(name, (offset, ty))| ParsedTypeItem::Field { name, ty, offset }),
    );
    let cpp_value = just(Token::Sharp)
        .then(just(Token::Ident("cpp_value")))
        .ignore_then(select! {
            Token::Str(c) => c,
        })
        .then(select! {
            Token::Str(c) => c,
        })
        .map(|x| ParsedTypeItem::CppValue {
            field: x.0,
            cpp_type: x.1,
        });
    let cpp_heap_allocated = just(Token::Sharp)
        .then(just(Token::Ident("cpp_heap_allocated")))
        .ignore_then(select! {
            Token::Str(c) => c,
        })
        .map(|cpp_type| ParsedTypeItem::CppHeapAllocated { cpp_type });
    let cpp_ref = just(Token::Sharp)
        .then(just(Token::Ident("cpp_ref")))
        .ignore_then(select! {
            Token::Str(c) => c,
        })
        .map(|x| ParsedTypeItem::CppRef { cpp_type: x });
    let cpp_stack_owned = just(Token::Sharp)
        .then(just(Token::Ident("cpp_stack_owned")))
        .ignore_then(select! {
            Token::Str(c) => c,
        })
        .then(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(|(cpp_type, props)| ParsedTypeItem::CppStackOwned { cpp_type, props });

    let variant = just(Token::Ident("variant"))
        .ignore_then(select! { Token::Ident(c) => c })
        .then(
            spanned(
                choice((non_exhaustive.clone(), field.clone())).then_ignore(just(Token::Semicolon)),
            )
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::BraceOpen), just(Token::BraceClose)),
        )
        .map(|(name, items)| ParsedTypeItem::Variant { name, items });

    recursive(|item| {
        let inner_item = choice((
            layout.boxed(),
            traits.boxed(),
            non_exhaustive.boxed(),
            constructor.boxed(),
            field.boxed(),
            cpp_value.boxed(),
            cpp_heap_allocated.boxed(),
            cpp_ref.boxed(),
            cpp_stack_owned.boxed(),
            method()
                .then(
                    just(Token::KwUse)
                        .ignore_then(path())
                        .map(Some)
                        .or(empty().to(None))
                        .boxed(),
                )
                .then(
                    just(Token::Ident("deref"))
                        .ignore_then(rust_type())
                        .map(Some)
                        .or(empty().to(None))
                        .boxed(),
                )
                .then(
                    just(Token::KwAs)
                        .ignore_then(select! { Token::Ident(c) => Some(c), })
                        .or(empty().to(None))
                        .boxed(),
                )
                .map(
                    |(((data, use_path), deref), cpp_name)| ParsedTypeItem::Method {
                        deref,
                        use_path,
                        data,
                        cpp_name,
                    },
                )
                .boxed(),
        ));

        let match_stmt = conditional_item::<_, CfgConditional<'a>, NItems>(item)
            .map(ParsedTypeItem::MatchOnCfg)
            .boxed();

        choice((
            match_stmt,
            variant,
            inner_item.then_ignore(just(Token::Semicolon)).boxed(),
        ))
    })
    .boxed()
}

fn type_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::KwType)
        .ignore_then(
            (select! { Token::Ident(c) => c })
                .map(ParsedTypeVar)
                .separated_by(just(Token::Comma))
                .at_least(1)
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .try_map_with(|vars, e: &mut MapExtra<_, ZngParserExtra>| {
                    if !e.state().unstable_features.template_types {
                        Err(Rich::custom(e.span(), "Template types are unstable. Enable them by using `#unstable(template_types)` at the top of the file."))
                    } else {
                        Ok(vars)
                    }
                })
                .or_not(),
        )
        .then(spanned(rust_type()))
        .then(
            spanned(inner_type_item())
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose)).boxed(),
        )
        .map(|((type_vars, ty), items)| ParsedItem::Type {
            ty,
            items,
            type_vars,
        })
        .boxed()
}

fn trait_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just(Token::KwTrait)
        .ignore_then(spanned(rust_trait(rust_type())))
        .then(
            method()
                .then_ignore(just(Token::Semicolon))
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(|(tr, methods)| ParsedItem::Trait { tr, methods })
        .boxed()
}

fn fn_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    spanned(method())
        .then_ignore(just(Token::Semicolon))
        .map(ParsedItem::Fn)
        .boxed()
}

fn additional_include_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::Sharp)
        .ignore_then(choice((
            just(Token::Ident("cpp_additional_includes"))
                .ignore_then(select! {
                    Token::Str(c) => ParsedItem::CppAdditionalInclude(c),
                    Token::RawStr(_, c) => ParsedItem::CppAdditionalInclude(c),
                })
                .boxed(),
            just(Token::Ident("convert_panic_to_exception"))
                .map_with(|_, extra| ParsedItem::ConvertPanicToException(extra.span()))
                .boxed(),
        )))
        .boxed()
}

fn extern_cpp_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    let safety = choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ));
    let function = safety
        .clone()
        .then(spanned(method()))
        .then_ignore(just(Token::Semicolon))
        .map(|(is_safe, method)| ParsedExternCppItem::Function { is_safe, method });
    let impl_block = just(Token::KwImpl)
        .ignore_then(
            rust_trait(rust_type())
                .then_ignore(just(Token::KwFor))
                .map(Some)
                .or(empty().to(None))
                .then(spanned(rust_type()))
                .boxed(),
        )
        .then(
            safety
                .then(method())
                .then_ignore(just(Token::Semicolon))
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(|((tr, ty), methods)| ParsedExternCppItem::Impl { tr, ty, methods });
    just(Token::KwExtern)
        .then(just(Token::Str("C++")))
        .ignore_then(
            function
                .or(impl_block)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(ParsedItem::ExternCpp)
        .boxed()
}

fn unstable_feature<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just([Token::Sharp, Token::Ident("unstable")])
        .ignore_then(
            select! { Token::Ident(feat) => feat }
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .try_map_with(|feat, e| match feat {
                    "cfg_match" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.cfg_match = true;
                        Ok(ParsedItem::UnstableFeature("cfg_match"))
                    }
                    "cfg_if" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.cfg_if = true;
                        Ok(ParsedItem::UnstableFeature("cfg_if"))
                    }
                    "template_types" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.template_types = true;
                        Ok(ParsedItem::UnstableFeature("template_types"))
                    }
                    _ => Err(Rich::custom(
                        e.span(),
                        format!("unknown unstable feature '{feat}'"),
                    )),
                }),
        )
        .boxed()
}

fn item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    recursive(|item| {
        choice((
            unstable_feature(),
            just(Token::KwMod)
                .ignore_then(path())
                .then(
                    item.clone()
                        .repeated()
                        .collect::<Vec<_>>()
                        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                        .boxed(),
                )
                .map(|(path, items)| ParsedItem::Mod { path, items })
                .boxed(),
            type_item(),
            trait_item(),
            extern_cpp_item(),
            fn_item(),
            additional_include_item(),
            import_item(),
            module_import_item(),
            alias(),
            conditional_item::<_, CfgConditional<'a>, NItems>(item).map(ParsedItem::MatchOnCfg),
        ))
    })
    .boxed()
}

fn import_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just(Token::KwMerge)
        .ignore_then(select! {
            Token::Str(path) => path,
        })
        .then_ignore(just(Token::Semicolon))
        .map_with(|path, extra| {
            ParsedItem::Import(ParsedImportPath {
                path: std::path::PathBuf::from(path),
                span: extra.span(),
            })
        })
        .boxed()
}

fn module_import_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::KwImport)
        .ignore_then(select! { Token::Str(path) => path })
        .then_ignore(just(Token::Semicolon))
        .map_with(|path, extra| ParsedItem::ModuleImport {
            path: std::path::PathBuf::from(path),
            span: extra.span(),
        })
        .boxed()
}

fn path<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedPath<'a>, ZngParserExtra<'a>> + Clone {
    let start = choice((
        just(Token::ColonColon).to(ParsedPathStart::Absolute),
        just(Token::KwCrate)
            .then(just(Token::ColonColon))
            .to(ParsedPathStart::Crate),
        empty().to(ParsedPathStart::Relative),
    ));

    start
        .then(
            (select! {
                Token::Ident(c) => c,
            })
            .separated_by(just(Token::ColonColon))
            .at_least(1)
            .collect::<Vec<_>>()
            .boxed(),
        )
        .or(just(Token::KwCrate).to((ParsedPathStart::Crate, vec![])))
        .map_with(|(start, segments), extra| ParsedPath {
            start,
            segments,
            span: extra.span(),
        })
        .boxed()
}

impl<'a> conditional::BodyItem for crate::ParsedTypeItem<'a> {
    type Processed = Self;

    fn process(self, _ctx: &mut ParseContext) -> Self::Processed {
        self
    }
}

impl<'a> conditional::BodyItem for crate::ParsedItem<'a> {
    type Processed = ProcessedItemOrAlias<'a>;

    fn process(self, ctx: &mut ParseContext) -> Self::Processed {
        crate::process_parsed_item(self, ctx)
    }
}
