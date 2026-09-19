use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    path::Component,
};

#[cfg(not(test))]
use std::process::exit;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{input::MapExtra, prelude::*};
use itertools::{Either, Itertools};

use zngur_def::{
    AdditionalIncludes, ConvertPanicToException, CppHeapAllocated, CppRef, CppStackOwned, Import,
    LayoutPolicy, Merge, MergeFailure, ModuleImport, Mutability, PrimitiveRustType,
    RustPathAndGenerics, RustTrait, RustType, TypeVar, ZngurConstructor, ZngurExternCppFn,
    ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod, ZngurMethodDetails, ZngurMethodReceiver,
    ZngurSpec, ZngurTrait, ZngurType, ZngurVariant, ZngurWellknownTrait,
};

pub type Span = SimpleSpan<usize>;

/// Result of parsing a .zng file, containing both the spec and the list of all processed files.
#[derive(Debug)]
pub struct ParseResult {
    /// The parsed Zngur specification
    pub spec: ZngurSpec,
    /// All .zng files that were processed (main file + transitive imports)
    pub processed_files: Vec<std::path::PathBuf>,
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
    Cpp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPath<'a> {
    start: ParsedPathStart,
    segments: Vec<&'a str>,
    span: Span,
}

/// A path to some named entity in either the Rust or C++ universe -- today
/// always a namespace/module path, but named generally since a scope's base
/// may need to refer to other kinds of entities (e.g. structs) later.
#[derive(Debug, Clone)]
enum EntityPath {
    /// If initial element is "crate", interpreted as crate relative.
    /// Otherwise, understood to be a global path.
    Rust(Vec<String>),
    Cpp(Vec<String>),
}

impl EntityPath {
    fn rust<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Rust(segments.into_iter().map(str::to_owned).collect())
    }

    fn cpp<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Cpp(segments.into_iter().map(str::to_owned).collect())
    }

    fn crate_relative<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::rust(["crate"].into_iter().chain(segments))
    }

    fn child(&self, name: &str) -> EntityPath {
        let mut child = self.clone();
        match &mut child {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => v.push(name.to_owned()),
        }
        child
    }

    fn join<'s>(&self, extra: impl IntoIterator<Item = &'s str>) -> EntityPath {
        let mut joined = self.clone();
        match &mut joined {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => {
                v.extend(extra.into_iter().map(str::to_owned))
            }
        }
        joined
    }
}

#[derive(Debug, Clone)]
struct Scope<'a> {
    aliases: Vec<ParsedAlias<'a>>,
    base: EntityPath,
    type_vars: HashSet<ParsedTypeVar<'a>>,
}

impl<'a> Scope<'a> {
    /// Create a new root scope containing the specified aliases.
    fn new_root(aliases: Vec<ParsedAlias<'a>>) -> Scope<'a> {
        Scope {
            aliases,
            base: EntityPath::Rust(Default::default()),
            type_vars: Default::default(),
        }
    }

    /// Resolve a path according to the current scope.
    fn resolve_path(&self, path: ParsedPath<'a>) -> EntityPath {
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

    /// Return rust source text that references `target` from within `self`.
    fn reference_to(&self, target: &EntityPath) -> Option<String> {
        match (target, &self.base) {
            // Rust paths don't use self.base
            (EntityPath::Rust(v), _) => Some(
                v.iter()
                    .map(|s| {
                        if s == "crate" {
                            s.clone()
                        } else {
                            format!("::{s}")
                        }
                    })
                    .collect(),
            ),
            // Cannot reference a Cpp path from a Rust path because we don't
            // know where the generated rust code will end up.
            (EntityPath::Cpp(_), EntityPath::Rust(_)) => None,
            // Cpp paths can reference other Cpp paths, but super may be needed.
            (EntityPath::Cpp(target_segs), EntityPath::Cpp(self_segs)) => {
                let common = self_segs
                    .iter()
                    .zip(target_segs)
                    .take_while(|(a, b)| a == b)
                    .count();
                let ups = std::iter::repeat("super").take(self_segs.len() - common);
                let downs = target_segs[common..].iter().map(String::as_str);
                Some(ups.chain(downs).join("::"))
            }
        }
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
    fn to_zngur(self, base: &EntityPath) -> EntityPath {
        match self.start {
            ParsedPathStart::Absolute => EntityPath::rust(self.segments),
            ParsedPathStart::Relative => base.join(self.segments),
            ParsedPathStart::Crate => EntityPath::crate_relative(self.segments),
            ParsedPathStart::Cpp => EntityPath::cpp(self.segments),
        }
    }

    fn matches_alias(&self, alias: &ParsedAlias<'_>) -> bool {
        self.start == ParsedPathStart::Relative
            && self
                .segments
                .first()
                .is_some_and(|part| *part == alias.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlias<'a> {
    name: &'a str,
    path: ParsedPath<'a>,
    span: Span,
}

impl ParsedAlias<'_> {
    fn expand(&self, path: &ParsedPath<'_>, base: &EntityPath) -> Option<EntityPath> {
        if path.matches_alias(self) {
            let extra = path.segments.iter().skip(1).copied();
            Some(self.path.clone().to_zngur(base).join(extra))
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

fn checked_merge<T, U>(src: T, dst: &mut U, span: Span, ctx: &mut ParseContext)
where
    T: Merge<U>,
{
    match src.merge(dst) {
        Ok(()) => {}
        Err(e) => match e {
            MergeFailure::Conflict(s) => {
                ctx.add_error_str(&s, span);
            }
        },
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
                let is_root = matches!(scope.base, EntityPath::Rust(ref v) if v.is_empty());
                // Only a relative mod path composes onto the enclosing module;
                // every anchored start (`c++::`, `crate::`, `::`) discards it,
                // so nesting one silently ignores the surrounding module. Allow
                // anchored paths only at the file root and require nested mods
                // to be relative.
                let anchor = match path.start {
                    ParsedPathStart::Cpp => Some("c++::"),
                    ParsedPathStart::Crate => Some("crate::"),
                    ParsedPathStart::Absolute => Some("::"),
                    ParsedPathStart::Relative => None,
                };
                if let Some(anchor) = anchor
                    && !is_root
                {
                    ctx.add_error_str(
                        &format!(
                            "`{anchor}` modules can only appear at the top level of a file, not nested inside another module"
                        ),
                        path.span,
                    );
                } else {
                    let sub_scope = scope.sub_scope(&aliases, path);
                    for item in items {
                        item.add_to_zngur_spec(r, &sub_scope, ctx);
                    }
                }
            }
            ProcessedItem::Import(path) => {
                if path.path.is_absolute() {
                    ctx.add_error_str("Absolute paths imports are not supported.", path.span)
                }
                match path.path.components().next() {
                    Some(Component::CurDir) | Some(Component::ParentDir) => {
                        r.imports.push(Import(path.path));
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
                while let Some(item) = to_process.pop() {
                    let item_span = item.span;
                    let item = item.inner;
                    match item {
                        ParsedTypeItem::Layout(span, p) => {
                            layout = Some(match p {
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
                            });
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
                            constructor = Some(ZngurConstructor {
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
                            });
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
                                        fields.push(ZngurField {
                                            name: name.to_owned(),
                                            ty: ty.to_zngur(scope),
                                            offset,
                                        });
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
                            fields.push(ZngurField {
                                name: name.to_owned(),
                                ty: ty.to_zngur(scope),
                                offset,
                            });
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
                            let use_path = use_path.and_then(|x| {
                                let span = x.span;
                                let target = scope.resolve_path(x);
                                let reference = scope.reference_to(&target);
                                if reference.is_none() {
                                    ctx.add_error_str(
                                        "cannot reference a c++::-only path from a Rust-only scope",
                                        span,
                                    );
                                }
                                reference
                            });
                            methods.push(ZngurMethodDetails {
                                data: data.to_zngur(scope),
                                use_path,
                                deref,
                                cpp_name: cpp_name.map(|s| s.to_owned()),
                            });
                        }
                        ParsedTypeItem::CppValue { field: _, cpp_type } => {
                            ctx.add_warning_str(
                                "#cpp_value is deprecated; use #cpp_heap_allocated instead",
                                item_span,
                            );
                            cpp_heap_allocated = Some(CppHeapAllocated(cpp_type.to_owned()));
                        }
                        ParsedTypeItem::CppHeapAllocated { cpp_type } => {
                            cpp_heap_allocated = Some(CppHeapAllocated(cpp_type.to_owned()));
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
                            cpp_ref = Some(CppRef(cpp_type.to_owned()));
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
                            cpp_stack_owned = Some(CppStackOwned {
                                cpp_type: cpp_type.to_owned(),
                                size,
                                align,
                            });
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
                                ctx.filename().to_string(),
                                span.start,
                            )
                            .with_message("Duplicate layout policy found for unsized type.")
                            .with_label(
                                Label::new((ctx.filename().to_string(), span.start..span.end))
                                    .with_message(
                                        "Unsized types have implicit layout policy, remove this.",
                                    )
                                    .with_color(Color::Red),
                            )
                            .with_label(
                                Label::new((
                                    ctx.filename().to_string(),
                                    is_unsized.span.start..is_unsized.span.end,
                                ))
                                .with_message("Type declared as unsized here.")
                                .with_color(Color::Blue),
                            )
                            .finish(),
                        )
                    }
                    layout = Some(LayoutPolicy::OnlyByRef);
                }
                let zngur_type = ZngurType {
                    ty: ty.inner.to_zngur(scope),
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
                        filename: ctx.filename().to_owned(),
                        span: ty.span,
                    });
                } else {
                    r.ty_to_locations
                        .entry(zngur_type.ty.clone())
                        .or_default()
                        .push((ctx.filename().to_owned(), ty.span.start..ty.span.end));
                    checked_merge(zngur_type, &mut r.spec, ty.span, ctx);
                }
            }
            ProcessedItem::Trait { tr, methods } => {
                checked_merge(
                    ZngurTrait {
                        tr: tr.inner.to_zngur(scope),
                        methods: methods.into_iter().map(|m| m.to_zngur(scope)).collect(),
                    },
                    &mut r.spec,
                    tr.span,
                    ctx,
                );
            }
            ProcessedItem::Fn(f) => {
                let method = f.inner.to_zngur(scope);
                let path = match scope.base.child(&method.name) {
                    EntityPath::Rust(v) => v,
                    EntityPath::Cpp(_) => {
                        ctx.add_error_str(
                            "a free function cannot be declared inside a c++:: scope",
                            f.span,
                        );
                        return;
                    }
                };
                checked_merge(
                    ZngurFn {
                        path: RustPathAndGenerics {
                            path,
                            generics: method.generics,
                            named_generics: vec![],
                        },
                        inputs: method.inputs,
                        output: method.output,
                    },
                    &mut r.spec,
                    f.span,
                    ctx,
                );
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
                None => match scope.resolve_path(s.path.clone()) {
                    EntityPath::Cpp(segs) => RustType::Cpp(segs),
                    EntityPath::Rust(_) => RustType::Adt(s.to_zngur(scope)),
                },
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
            path: match scope.resolve_path(self.path) {
                EntityPath::Rust(v) => v,
                EntityPath::Cpp(_) => todo!(
                    "a c++::-only path was used somewhere that can't support it (e.g. as a trait); needs a proper user-facing error instead of this panic"
                ),
            },
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

/// A diagnostic report, tagged with whether it should abort the parse.
struct ReportEntry<'b> {
    fatal: bool,
    report: Report<'b, (String, std::ops::Range<usize>)>,
}

struct ParseContext<'a, 'b> {
    path: std::path::PathBuf,
    text: &'a str,
    depth: usize,
    reports: Vec<ReportEntry<'b>>,
    source_cache: std::collections::HashMap<std::path::PathBuf, String>,
    /// All .zng files processed during parsing (main file + imports)
    processed_files: Vec<std::path::PathBuf>,
    cfg_provider: Box<dyn RustCfgProvider>,
}

impl<'a, 'b> ParseContext<'a, 'b> {
    fn new(path: std::path::PathBuf, text: &'a str, cfg: Box<dyn RustCfgProvider>) -> Self {
        let processed_files = vec![path.clone()];
        Self {
            path,
            text,
            depth: 0,
            reports: Vec::new(),
            source_cache: HashMap::new(),
            processed_files,
            cfg_provider: cfg,
        }
    }

    fn with_depth(
        path: std::path::PathBuf,
        text: &'a str,
        depth: usize,
        cfg: Box<dyn RustCfgProvider>,
    ) -> Self {
        let processed_files = vec![path.clone()];
        Self {
            path,
            text,
            depth,
            reports: Vec::new(),
            source_cache: HashMap::new(),
            processed_files,
            cfg_provider: cfg,
        }
    }

    fn filename(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    /// Every known source file (this file plus every imported file already
    /// processed), paired with its text, for ariadne's `sources()` cache. Needed so
    /// that a report whose span points into an imported file can still be rendered
    /// correctly after that file's `ParseContext` has been merged into this one.
    fn all_sources(&self) -> impl Iterator<Item = (String, &str)> {
        [(self.filename().to_string(), self.text)]
            .into_iter()
            .chain(self.source_cache.iter().map(|(path, text)| {
                (
                    path.file_name().unwrap().to_str().unwrap().to_string(),
                    text.as_str(),
                )
            }))
    }

    /// Renders a single report to text (stripped of ANSI color codes).
    fn render_report(&self, report: &Report<'b, (String, std::ops::Range<usize>)>) -> String {
        let mut buf = Vec::new();
        report.write(sources(self.all_sources()), &mut buf).unwrap();
        String::from_utf8(strip_ansi_escapes::strip(buf)).unwrap()
    }

    fn add_fatal_report(&mut self, report: Report<'b, (String, std::ops::Range<usize>)>) {
        self.reports.push(ReportEntry {
            fatal: true,
            report,
        });
    }
    fn add_errors<'err_src>(&mut self, errs: impl Iterator<Item = Rich<'err_src, String>>) {
        let filename = self.filename().to_string();
        self.reports.extend(errs.map(|e| {
            let report = Report::build(ReportKind::Error, &filename, e.span().start)
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), e.span().into_range()))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), span.into_range()))
                        .with_message(format!("while parsing this {}", label))
                        .with_color(Color::Yellow)
                }))
                .finish();
            ReportEntry {
                fatal: true,
                report,
            }
        }));
    }

    fn add_error_str(&mut self, error: &str, span: Span) {
        self.add_errors([Rich::custom(span, error)].into_iter());
    }

    /// Records a non-fatal warning as an ariadne diagnostic pointing at `span`.
    /// Rendering is deferred until dispatch time (see `render_report`).
    fn add_warning_str(&mut self, warning: &str, span: Span) {
        let filename = self.filename().to_string();
        let report = Report::build(ReportKind::Warning, &filename, span.start)
            .with_message(warning)
            .with_label(
                Label::new((filename.clone(), span.into_range()))
                    .with_message(warning)
                    .with_color(Color::Yellow),
            )
            .finish();
        self.reports.push(ReportEntry {
            fatal: false,
            report,
        });
    }

    fn consume_from(&mut self, mut other: ParseContext<'_, 'b>) {
        self.processed_files.append(&mut other.processed_files);
        self.reports.extend(other.reports);
        // Always cache the source in case errors come up in post-processing
        self.source_cache.insert(other.path, other.text.to_string());
        self.source_cache.extend(other.source_cache);
    }

    fn has_errors(&self) -> bool {
        self.reports.iter().any(|entry| entry.fatal)
    }

    #[cfg(test)]
    fn emit_ariadne_errors(&self) -> ! {
        let mut r = Vec::<u8>::new();
        for entry in self.reports.iter().filter(|entry| entry.fatal) {
            entry
                .report
                .write(sources(self.all_sources()), &mut r)
                .unwrap();
        }
        std::panic::resume_unwind(Box::new(tests::ErrorText({
            let s = String::from_utf8(strip_ansi_escapes::strip(r)).unwrap();
            eprintln!("{s}");
            s
        })));
    }

    #[cfg(not(test))]
    fn emit_ariadne_errors(&self) -> ! {
        for entry in self.reports.iter().filter(|entry| entry.fatal) {
            entry.report.eprint(sources(self.all_sources())).unwrap();
        }
        exit(101);
    }

    fn get_config_provider(&self) -> &dyn RustCfgProvider {
        self.cfg_provider.as_ref()
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
struct DefaultImportResolver;

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

impl<'a> ParsedZngFile<'a> {
    fn parse_into(
        zngur: &mut ZngurSpecBuilder,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let (tokens, errs) = lexer().parse(ctx.text).into_output_errors();
        let Some(tokens) = tokens else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            ctx.emit_ariadne_errors();
        };
        let tokens: ParserInput<'_> = tokens.as_slice().map(
            (ctx.text.len()..ctx.text.len()).into(),
            Box::new(|(t, s)| (t, s)),
        );
        let (ast, errs) = file_parser()
            .map_with(|ast, extra| (ast, extra.span()))
            .parse_with_state(tokens, &mut extra::SimpleState(ZngParserState::default()))
            .into_output_errors();
        let Some(ast) = ast else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            ctx.emit_ariadne_errors();
        };

        let (aliases, items) = partition_parsed_items(
            ast.0
                .0
                .into_iter()
                .map(|item| process_parsed_item(item, ctx)),
        );
        ProcessedZngFile::new(aliases, items).into_zngur_spec(zngur, ctx);

        if let Some(dirname) = ctx.path.to_owned().parent() {
            for import in std::mem::take(&mut zngur.imports) {
                match resolver.resolve_import(dirname, &import.0) {
                    Ok(text) => {
                        let mut nested_ctx = ParseContext::with_depth(
                            dirname.join(&import.0),
                            &text,
                            ctx.depth + 1,
                            ctx.get_config_provider().clone_box(),
                        );
                        Self::parse_into(zngur, &mut nested_ctx, resolver);
                        ctx.consume_from(nested_ctx);
                    }
                    Err(_) => {
                        // TODO: emit a better error. How should we get a span here?
                        // I'd like to avoid putting a ParsedImportPath in ZngurSpec, and
                        // also not have to pass a filename to add_to_zngur_spec.
                        ctx.add_fatal_report(
                            Report::build(ReportKind::Error, ctx.filename(), 0)
                                .with_message(format!(
                                    "Import path not found: {}",
                                    import.0.display()
                                ))
                                .finish(),
                        );
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
        path: std::path::PathBuf,
        cfg: Box<dyn RustCfgProvider>,
        warning_sink: &dyn Fn(&str),
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        zngur.spec.rust_cfg.extend(cfg.get_cfg_pairs());
        zngur.spec.rust_cfg.sort();
        let text = std::fs::read_to_string(&path).unwrap();
        let mut ctx = ParseContext::new(path.clone(), &text, cfg.clone_box());
        Self::parse_into(&mut zngur, &mut ctx, &DefaultImportResolver);
        let spec = zngur.to_zngur(&mut ctx);
        for entry in ctx.reports.iter().filter(|entry| !entry.fatal) {
            warning_sink(&ctx.render_report(&entry.report));
        }
        if ctx.has_errors() {
            // add report of cfg values used
            ctx.add_fatal_report(
                Report::build(
                    ReportKind::Custom("cfg values", ariadne::Color::Green),
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    0,
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
            ctx.emit_ariadne_errors();
        }
        ParseResult {
            spec,
            processed_files: ctx.processed_files,
        }
    }

    /// Parse a .zng file from a string. Mainly useful for testing.
    #[cfg(test)]
    pub fn parse_str(
        text: &str,
        cfg: impl RustCfgProvider + 'static,
        warning_sink: impl FnMut(&str),
    ) -> ParseResult {
        Self::parse_str_with_resolver(text, cfg, &DefaultImportResolver, warning_sink)
    }

    #[cfg(test)]
    pub(crate) fn parse_str_with_resolver(
        text: &str,
        cfg: impl RustCfgProvider + 'static,
        resolver: &impl ImportResolver,
        mut warning_sink: impl FnMut(&str),
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        let mut ctx = ParseContext::new(std::path::PathBuf::from("test.zng"), text, Box::new(cfg));
        Self::parse_into(&mut zngur, &mut ctx, resolver);
        let spec = zngur.to_zngur(&mut ctx);
        for entry in ctx.reports.iter().filter(|entry| !entry.fatal) {
            warning_sink(&ctx.render_report(&entry.report));
        }
        if ctx.has_errors() {
            ctx.emit_ariadne_errors();
        }
        ParseResult {
            spec,
            processed_files: ctx.processed_files,
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
    filename: String,
    span: Span,
}

#[derive(Default)]
struct ZngurSpecBuilder {
    spec: ZngurSpec,
    templates: Vec<TemplateDef>,
    ty_to_locations: HashMap<RustType, Vec<(String, std::ops::Range<usize>)>>,
    imports: Vec<Import>,
}

impl ZngurSpecBuilder {
    fn to_zngur(self, ctx: &mut ParseContext) -> ZngurSpec {
        let ZngurSpecBuilder {
            mut spec,
            templates,
            imports: _,
            mut ty_to_locations,
        } = self;
        for ty in &mut spec.types {
            let mut template_locations = Vec::new();
            for template in &templates {
                if let Some(template_match) = try_match_template(&ty.ty, &template.ty) {
                    let location = (
                        template.filename.clone(),
                        template.span.start..template.span.end,
                    );
                    if let Err(e) = template_match.merge(ty) {
                        let MergeFailure::Conflict(e) = e;
                        ctx.add_fatal_report(
                            Report::build(ReportKind::Error, &template.filename, 0)
                                .with_message(format!(
                                    "Failed to apply template {} to type {}: {}",
                                    template.ty.ty, ty.ty, e
                                ))
                                .with_label(
                                    Label::new(location)
                                        .with_message("Template defined here")
                                        .with_color(Color::Blue),
                                )
                                .finish(),
                        );
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
                let mut report = Report::build(ReportKind::Error, "", 0).with_message(format!(
                    "No layout policy found for type {}. \
    Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.",
                    ty.ty
                ));
                for location in ty_to_locations.remove(&ty.ty).unwrap_or_default() {
                    report = report.with_label(
                        Label::new(location)
                            .with_message("Type defined here")
                            .with_color(Color::Blue),
                    );
                }
                for location in template_locations {
                    report = report.with_label(
                        Label::new(location)
                            .with_message("Matching template defined here")
                            .with_color(Color::Blue),
                    );
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
    CppPathStart,
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
            Token::CppPathStart => write!(f, "c++"),
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
            just("c++").to(Token::CppPathStart),
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

    let rust_trait = fn_trait.or(rust_path_and_generics(rust_type)
        .try_map_with(|pg, extra| {
            if pg.path.start == ParsedPathStart::Cpp {
                Err(Rich::custom(
                    extra.span(),
                    "`c++::` paths cannot be used as a trait",
                ))
            } else {
                Ok(pg)
            }
        })
        .map(ParsedRustTrait::Normal));
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
                        .ignore_then(non_cpp_path("a method's `use` path"))
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
        just(Token::CppPathStart)
            .then(just(Token::ColonColon))
            .to(ParsedPathStart::Cpp),
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

fn non_cpp_path<'a>(
    context: &'static str,
) -> impl Parser<'a, ParserInput<'a>, ParsedPath<'a>, ZngParserExtra<'a>> + Clone {
    path().try_map_with(move |p, extra| {
        if p.start == ParsedPathStart::Cpp {
            Err(Rich::custom(
                extra.span(),
                format!("`c++::` paths cannot be used in {context}"),
            ))
        } else {
            Ok(p)
        }
    })
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
