use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    ops::{Deref, DerefMut},
    path::Component,
};

use ariadne::{Color, Label, Report, ReportKind};
use chumsky::{input::MapExtra, prelude::*};
use indexmap::IndexMap;
use itertools::{Either, Itertools};

use zngur_def::{
    AdditionalIncludes, ConflictSource, ConvertPanicToException, CppHeapAllocated, CppRef,
    CppStackOwned, Import, LayoutPolicy, Merge, MergeFailure, ModuleImport, Mutability,
    PrimitiveRustType, RustPathAndGenerics, RustTrait, RustType, TypeVar, ZngurConstructor,
    ZngurExternCppFn, ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod, ZngurMethodDetails,
    ZngurMethodReceiver, ZngurSpec, ZngurTrait, ZngurType, ZngurVariant, ZngurWellknownTrait,
    cpp_handle_field_name, cpp_handle_keyword,
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
mod dfs;
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
    /// Creates a Rust entity path from its segments (either crate-relative if
    /// the first segment is `"crate"`, or globally rooted otherwise).
    fn rust<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Rust(segments.into_iter().map(str::to_owned).collect())
    }

    /// Creates a `c++::` entity path from its segments (excluding the `c++::` prefix).
    fn cpp<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Cpp(segments.into_iter().map(str::to_owned).collect())
    }

    /// Creates a crate-relative Rust entity path (`crate::<segments>`).
    fn crate_relative<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::rust(["crate"].into_iter().chain(segments))
    }

    /// Returns a new path with `name` appended as a single trailing segment,
    /// preserving whether this path lives in the Rust or C++ universe.
    fn child(&self, name: &str) -> EntityPath {
        let mut child = self.clone();
        match &mut child {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => v.push(name.to_owned()),
        }
        child
    }

    /// Returns a new path with `extra` segments appended to `self`,
    /// preserving whether this path lives in the Rust or C++ universe.
    fn join<'s>(&self, extra: impl IntoIterator<Item = &'s str>) -> EntityPath {
        let mut joined = self.clone();
        match &mut joined {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => {
                v.extend(extra.into_iter().map(str::to_owned))
            }
        }
        joined
    }

    /// Return Rust source text that references a Rust `target` path.
    /// `EntityPath::Cpp` targets and the empty root `EntityPath::Rust` cannot be referenced here.
    fn rust_reference(&self) -> Option<String> {
        match self {
            EntityPath::Rust(v) if v.is_empty() => None,
            EntityPath::Rust(v) => {
                if v.first().is_some_and(|s| s == "crate") {
                    Some(v.join("::"))
                } else {
                    Some(format!("::{}", v.join("::")))
                }
            }
            EntityPath::Cpp(_) => None,
        }
    }
}

/// A `use <path> as <name>;` alias whose target path has been resolved against
/// the lexical scope (base module path and enclosing aliases) of its declaring module.
///
/// Storing the already-resolved `EntityPath` ensures that when an alias is inherited
/// into a nested `mod`, it continues to refer to the entity from its declaring module
/// rather than being re-evaluated against the nested module's base path.
#[derive(Debug, Clone)]
struct ResolvedAlias<'a> {
    /// The short identifier introduced by `use ... as <name>;`.
    name: &'a str,
    /// The fully resolved Rust or C++ entity path that `name` expands to.
    target: EntityPath,
}

impl ResolvedAlias<'_> {
    /// Returns `true` if `path` is a relative path whose first segment equals `self.name`.
    ///
    /// Root-prefixed (`::...`), `crate::`, and `c++::` paths never match an alias.
    fn path_uses_alias(&self, path: &ParsedPath<'_>) -> bool {
        path.try_as_relative()
            .map(RelativePath::split_base)
            .is_some_and(|(head, _)| head == self.name)
    }

    /// If `path` uses this alias, replaces that leading segment with `self.target`
    /// and appends any remaining segments from `path` (e.g., resolving `sub::Foo` via
    /// `use c++::a::b as sub;` produces `c++::a::b::Foo`).
    fn try_resolve(&self, path: &ParsedPath<'_>) -> Option<EntityPath> {
        let (head, rest) = path.try_as_relative().map(RelativePath::split_base)?;
        (head == self.name).then(|| self.target.join(rest.iter().copied()))
    }
}

/// Resolves the `use <path> as <name>;` aliases declared directly within a single module.
///
/// Aliases in the same module are order-independent (an earlier `use` may reference an
/// alias defined later in the same module) and shadow `parent_aliases` inherited from
/// enclosing modules. Cycles among same-module aliases are detected via
/// [`dfs::DfsCycleDetector`].
struct ModuleAliasResolver<'a, 'b> {
    /// Deduplicated local aliases in declaration order, keyed by alias name.
    local_aliases: &'b IndexMap<&'a str, &'b ParsedAlias<'a>>,
    /// Base path of the module in which `local_aliases` were declared.
    base: &'b EntityPath,
    /// Already-resolved aliases inherited from enclosing modules (innermost first).
    parent_aliases: &'b [ResolvedAlias<'a>],
}

impl<'a, 'b> ModuleAliasResolver<'a, 'b> {
    /// Resolves every local alias in declaration order.
    fn resolve_all(&self, ctx: &mut ParseContext) -> Vec<ResolvedAlias<'a>> {
        let mut dfs = dfs::DfsCycleDetector::default();
        (0..self.local_aliases.len())
            .map(|idx| {
                let (&name, _) = self
                    .local_aliases
                    .get_index(idx)
                    .expect("valid local alias index");
                let target = dfs
                    .with_node(idx, |dfs| self.resolve_alias_at(idx, dfs, ctx))
                    .expect("initial node on an empty DFS stack cannot form a cycle");
                ResolvedAlias { name, target }
            })
            .collect()
    }

    /// Resolves the target path of `self.local_aliases[idx]` using three precedence rules:
    /// 1. **Same-module alias**: If the path is relative and its leading segment names
    ///    another alias in `self.local_aliases`, recursively resolve that local alias
    ///    (or report a cycle error if it is already on the active DFS stack) and append
    ///    any trailing segments.
    /// 2. **Inherited parent alias**: Otherwise, if the path matches an alias in
    ///    `self.parent_aliases`, resolve it via that parent alias.
    /// 3. **Module base path**: Otherwise, resolve the path directly against `self.base`.
    fn resolve_alias_at(
        &self,
        idx: usize,
        dfs: &mut dfs::DfsCycleDetector,
        ctx: &mut ParseContext,
    ) -> EntityPath {
        let (_, &alias) = self
            .local_aliases
            .get_index(idx)
            .expect("valid local alias index");

        // If alias path matches another alias locally...
        if let Some((head, rest)) = alias.path.try_as_relative().map(RelativePath::split_base)
            && let Some(target_idx) = self.local_aliases.get_index_of(head)
        {
            match dfs.with_node(target_idx, |dfs| {
                self.resolve_alias_at(target_idx, dfs, ctx)
            }) {
                Ok(prefix) => prefix.join(rest.iter().copied()),
                Err(dfs::DfsCycle {
                    newly_detected_head,
                }) => {
                    if let Some(head_idx) = newly_detected_head {
                        let (_, &head_alias) = self
                            .local_aliases
                            .get_index(head_idx)
                            .expect("valid cycle head index");
                        ctx.add_error_str(
                            &format!("cyclic `use` alias definition `{}`", head_alias.name),
                            head_alias.span,
                        );
                    }
                    // The cycle above will trigger an error. Resolve against self.base just to
                    // allow collecting all errors in the module for better diagnostics.
                    alias.path.to_zngur(self.base)
                }
            }
        } else if let Some(expanded) = self
            // Try matching against a parent alias
            .parent_aliases
            .iter()
            .find_map(|parent| parent.try_resolve(&alias.path))
        {
            expanded
        } else {
            // Otherwise, the alias is final (refers to the module itself)
            alias.path.to_zngur(self.base)
        }
    }
}

#[derive(Debug, Clone)]
struct Scope<'a> {
    aliases: Vec<ResolvedAlias<'a>>,
    base: EntityPath,
    type_vars: HashSet<ParsedTypeVar<'a>>,
}

impl<'a> Scope<'a> {
    /// Create a new root scope containing the specified aliases.
    fn new_root(aliases: Vec<ParsedAlias<'a>>, ctx: &mut ParseContext) -> Scope<'a> {
        let base = EntityPath::Rust(Default::default());
        let resolved_aliases = Self::resolve_aliases(&aliases, &base, &[], ctx);
        Scope {
            aliases: resolved_aliases,
            base,
            type_vars: Default::default(),
        }
    }

    /// Resolves `new_aliases` declared in a module with `base` path and combines them
    /// with `parent_aliases` inherited from enclosing modules.
    ///
    /// Local aliases precede `parent_aliases` in the returned `Vec` so that inner
    /// module aliases shadow outer ones during [`Scope::resolve_path`].
    fn resolve_aliases(
        new_aliases: &[ParsedAlias<'a>],
        base: &EntityPath,
        parent_aliases: &[ResolvedAlias<'a>],
        ctx: &mut ParseContext,
    ) -> Vec<ResolvedAlias<'a>> {
        let unique_aliases = Self::dedup_module_aliases(new_aliases, ctx);
        let resolver = ModuleAliasResolver {
            local_aliases: &unique_aliases,
            base,
            parent_aliases,
        };
        let mut resolved = resolver.resolve_all(ctx);
        resolved.extend_from_slice(parent_aliases);
        resolved
    }

    /// Collects the aliases declared in a single module into an order-preserving map
    /// keyed by alias name, emitting an error whenever two `use` items in the same
    /// module define the same alias name.
    fn dedup_module_aliases<'b>(
        aliases: &'b [ParsedAlias<'a>],
        ctx: &mut ParseContext,
    ) -> IndexMap<&'a str, &'b ParsedAlias<'a>> {
        let mut unique = IndexMap::with_capacity(aliases.len());
        for alias in aliases {
            match unique.entry(alias.name) {
                indexmap::map::Entry::Vacant(vacant) => {
                    vacant.insert(alias);
                }
                indexmap::map::Entry::Occupied(first) => {
                    Self::report_duplicate_alias(alias, first.get(), ctx);
                }
            }
        }
        unique
    }

    /// Reports a diagnostic when `duplicate` redefines `first` in the same module.
    fn report_duplicate_alias(
        duplicate: &ParsedAlias<'_>,
        first: &ParsedAlias<'_>,
        ctx: &mut ParseContext,
    ) {
        let dup_span = ctx.report_span_for_range(duplicate.span.into_range());
        let prev_span = ctx.report_span_for_range(first.span.into_range());
        let msg = format!(
            "duplicate `use` alias `{}` in the same module",
            duplicate.name
        );
        let report = Report::build(ReportKind::Error, dup_span.clone())
            .with_message(&msg)
            .with_label(
                Label::new(dup_span)
                    .with_message(&msg)
                    .with_color(Color::Red),
            )
            .with_label(
                Label::new(prev_span)
                    .with_message("previously defined here")
                    .with_color(Color::Blue),
            )
            .finish();
        ctx.add_fatal_report(report);
    }

    fn has_alias(&self, path: &ParsedPath<'_>) -> bool {
        self.aliases.iter().any(|alias| alias.path_uses_alias(path))
    }

    /// Resolve a path according to the current scope.
    fn resolve_path(&self, path: &ParsedPath<'_>) -> EntityPath {
        // Check to see if the path refers to an alias:
        if let Some(expanded_alias) = self
            .aliases
            .iter()
            .find_map(|alias| alias.try_resolve(path))
        {
            expanded_alias
        } else {
            path.to_zngur(&self.base)
        }
    }

    fn report_invalid_cpp_path(
        &self,
        ctx: &mut ParseContext,
        path: &ParsedPath<'_>,
        message: &str,
        rust_kind_plural: &str,
    ) {
        let is_bare_relative = path.start == ParsedPathStart::Relative && !self.has_alias(path);
        let report_span = ctx.report_span_for_range(path.span.into_range());
        let mut report = Report::build(ReportKind::Error, report_span.clone())
            .with_message(message)
            .with_label(
                Label::new(report_span)
                    .with_message(message)
                    .with_color(Color::Red),
            );
        if is_bare_relative && matches!(self.base, EntityPath::Cpp(_)) {
            report = report.with_help(format!(
                "inside a `c++::` module, prefix Rust {rust_kind_plural} with `::` or `crate::`"
            ));
        }
        ctx.add_fatal_report(report.finish());
    }

    fn sub_scope(
        &self,
        new_aliases: &[ParsedAlias<'a>],
        nested_path: &ParsedPath<'_>,
        ctx: &mut ParseContext,
    ) -> Scope<'_> {
        let base = nested_path.to_zngur(&self.base);
        let mod_aliases = Self::resolve_aliases(new_aliases, &base, &self.aliases, ctx);

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

/// A borrowed view of a non-empty [`ParsedPath`] with [`ParsedPathStart::Relative`].
#[derive(Debug, Clone, Copy)]
struct RelativePath<'p, 'a> {
    base: &'a str,
    rest: &'p [&'a str],
}

impl<'p, 'a> RelativePath<'p, 'a> {
    /// Splits this relative path into its leading segment (`"foo"`) and any
    /// trailing segments (`&["bar"]` for `foo::bar`).
    fn split_base(self) -> (&'a str, &'p [&'a str]) {
        (self.base, self.rest)
    }
}

impl<'a> ParsedPath<'a> {
    /// Returns `Some(RelativePath)` if `self` is a relative path (`foo` or `foo::bar`),
    /// or `None` for `::`, `crate::`, and `c++::` paths (which never match aliases).
    fn try_as_relative(&self) -> Option<RelativePath<'_, 'a>> {
        if self.start == ParsedPathStart::Relative {
            let (&base, rest) = self.segments.split_first()?;
            Some(RelativePath { base, rest })
        } else {
            None
        }
    }

    fn to_zngur(&self, base: &EntityPath) -> EntityPath {
        let segments = self.segments.iter().copied();
        match self.start {
            ParsedPathStart::Absolute => EntityPath::rust(segments),
            ParsedPathStart::Relative => base.join(segments),
            ParsedPathStart::Crate => EntityPath::crate_relative(segments),
            ParsedPathStart::Cpp => EntityPath::cpp(segments),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlias<'a> {
    name: &'a str,
    path: ParsedPath<'a>,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedImportPath {
    path: std::path::PathBuf,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedItem<'a> {
    ConvertPanicToException(Span),
    CrateName(&'a str, Span),
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
    Fn {
        is_safe: Option<bool>,
        function: Spanned<ParsedFunction<'a>>,
    },
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
    CrateName(&'a str, Span),
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
    Fn {
        is_safe: Option<bool>,
        function: Spanned<ParsedFunction<'a>>,
    },
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
    // For the `#cpp_*` directives, `cpp_type` is `None` if the C++ type name is omitted, which is
    // allowed for `c++::` types: the name is the type's path.
    CppHeapAllocated {
        cpp_type: Option<&'a str>,
    },
    CppRef {
        cpp_type: Option<&'a str>,
    },
    CppStackOwned {
        cpp_type: Option<&'a str>,
        props: Option<Vec<(Spanned<&'a str>, usize)>>,
    },
    /// `#trivially_relocatable`.
    TriviallyRelocatable,
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedTypeItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParsedTypeVar<'a>(&'a str);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMethod<'a> {
    name: &'a str,
    is_safe: Option<bool>,
    receiver: ZngurMethodReceiver,
    generics: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedFunction<'a> {
    path: ParsedPath<'a>,
    signature: ParsedMethod<'a>,
}

impl ParsedMethod<'_> {
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> ZngurMethod {
        ZngurMethod {
            name: self.name.to_owned(),
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope, ctx))
                .collect(),
            receiver: self.receiver,
            inputs: self
                .inputs
                .into_iter()
                .map(|x| x.to_zngur(scope, ctx))
                .collect(),
            output: self.output.to_zngur(scope, ctx),
            is_safe: self.is_safe,
        }
    }
}

struct MergeContext {
    outer_ty: RustType,
    variant: Option<String>,
}

/// The C++ type name of a `#cpp_heap_allocated`, `#cpp_ref` or `#cpp_stack_owned` directive on
/// `ty`: the explicit one if given, otherwise the path of a `c++::` type (`c++::a::B` is `::a::B`).
fn cpp_type_name(
    explicit: Option<&str>,
    ty: &RustType,
    span: Span,
    ctx: &mut ParseContext,
) -> Option<String> {
    match (explicit, ty.cpp_spelling()) {
        (Some(name), _) => Some(name.to_owned()),
        (None, Some(name)) => Some(name),
        (None, None) => {
            ctx.add_error_str(
                "the C++ type name can only be omitted for `c++::` types, whose path is the name",
                span,
            );
            None
        }
    }
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
                    let sub_scope = scope.sub_scope(&aliases, &path, ctx);
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
            ProcessedItem::ModuleImport { path, span } => {
                let resolved_path = if ctx.depth > 0
                    && let Some((root_dir, current_dir)) =
                        ctx.module_root_dir.as_deref().zip(ctx.path.parent())
                {
                    rebase_relative_import_path(root_dir, current_dir, &path)
                } else {
                    path
                };
                r.spec.imported_modules.push(ModuleImport {
                    path: resolved_path,
                    crate_name: None,
                    cpp_types: vec![],
                });
                r.module_import_spans
                    .push(ctx.report_span_for_range(span.into_range()));
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
                let mut trivially_relocatable = false;
                let mut to_process = items;
                to_process.reverse(); // create a stack of items to process
                let check_size_align = |props: Vec<(Spanned<&str>, usize)>| {
                    let mut size = None;
                    let mut align = None;
                    let mut errors = vec![];
                    for (key, value) in props {
                        match key.inner {
                            "size" => size = Some(value),
                            "align" => {
                                if !value.is_power_of_two() {
                                    errors.push(("Alignment must be a power of two", key.span));
                                }
                                align = Some(value);
                            }
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
                let rust_ty = ty.inner.to_zngur(scope, ctx);
                while let Some(item) = to_process.pop() {
                    let item_span = item.span;
                    let item = item.inner;
                    match item {
                        ParsedTypeItem::Layout(span, p) => {
                            let l = match p {
                                ParsedLayoutPolicy::StackAllocated(p) => {
                                    check_size_align(p).map(|(size, align)| {
                                        LayoutPolicy::StackAllocated { size, align }
                                    })
                                }
                                ParsedLayoutPolicy::Conservative(p) => {
                                    check_size_align(p).map(|(size, align)| {
                                        LayoutPolicy::Conservative { size, align }
                                    })
                                }
                                ParsedLayoutPolicy::HeapAllocated => {
                                    Ok(LayoutPolicy::HeapAllocated)
                                }
                                ParsedLayoutPolicy::OnlyByRef => Ok(LayoutPolicy::OnlyByRef),
                            };
                            let l = match l {
                                Ok(l) => l,
                                Err(errs) => {
                                    for (msg, err_span) in errs {
                                        ctx.add_error_str(msg, err_span);
                                    }
                                    if layout.is_none() {
                                        layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                                    }
                                    if layout_span.is_none() {
                                        layout_span = Some(span);
                                    }
                                    continue;
                                }
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
                            for t in &tr {
                                ctx.record_span(
                                    &PartialSpanKey::WellknownTrait(rust_ty.clone(), t.inner),
                                    t.span.into_range(),
                                );
                            }
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
                                        .map(|(i, t)| (i.to_string(), t.to_zngur(scope, ctx)))
                                        .collect(),
                                    ParsedConstructorArgs::Named(t) => t
                                        .into_iter()
                                        .map(|(i, t)| (i.to_owned(), t.to_zngur(scope, ctx)))
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
                                            ty: ty.to_zngur(scope, ctx),
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
                                ty: ty.to_zngur(scope, ctx),
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
                                let deref_type = x.to_zngur(scope, ctx);
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
                                let target = scope.resolve_path(&x);
                                match target.rust_reference() {
                                    Some(reference) => Some(reference),
                                    None => {
                                        scope.report_invalid_cpp_path(
                                            ctx,
                                            &x,
                                            "`c++::` paths cannot be used in a method's `use` path",
                                            "paths",
                                        );
                                        None
                                    }
                                }
                            });
                            let method = ZngurMethodDetails {
                                data: data.to_zngur(scope, ctx),
                                use_path,
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
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppHeapAllocated { cpp_type } => {
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
                            let cpp = CppHeapAllocated(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppRef { cpp_type } => {
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
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
                            let cpp = CppRef(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_ref = Some(cpp);
                        }
                        ParsedTypeItem::CppStackOwned { cpp_type, props } => {
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
                            let has_legacy_props = props.is_some();
                            if let Some(props) = props {
                                let size = props
                                    .iter()
                                    .find(|(k, _)| k.inner == "size")
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| "X".to_string());
                                let align = props
                                    .iter()
                                    .find(|(k, _)| k.inner == "align")
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| "Y".to_string());
                                let report_span = ctx.report_span_for_range(item_span.into_range());
                                let report = Report::build(ReportKind::Error, report_span.clone())
                                    .with_message("Old syntax `#cpp_stack_owned` with layout properties is no longer supported.")
                                    .with_label(
                                        Label::new(report_span)
                                            .with_message("layout parameters no longer accepted here")
                                            .with_color(Color::Red),
                                    )
                                    .with_help(format!(
                                        "layout moved to its own directive: `#cpp_stack_owned \"{cpp_type}\"; #layout(size = {size}, align = {align});`"
                                    ))
                                    .finish();
                                ctx.add_fatal_report(report);
                            }
                            let cpp = CppStackOwned(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_stack_owned = Some(cpp);
                            if has_legacy_props && layout.is_none() {
                                layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                            }
                        }
                        ParsedTypeItem::TriviallyRelocatable => {
                            ctx.record_span(
                                &PartialSpanKey::TriviallyRelocatable(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            trivially_relocatable = true;
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
                    trivially_relocatable,
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
                    tr: tr.inner.to_zngur(scope, ctx),
                    methods: methods
                        .into_iter()
                        .map(|m| m.to_zngur(scope, ctx))
                        .collect(),
                };
                ctx.record_span(&trt, tr.span.into_range());
                checked_merge(trt, &mut r.spec, tr.span, ctx, None);
            }
            ProcessedItem::Fn {
                is_safe,
                function: f,
            } => {
                let function = f.inner;
                let method = function.signature.to_zngur(scope, ctx);
                let target = if function.path.start == ParsedPathStart::Relative
                    && function.path.segments.len() == 1
                {
                    scope.base.child(function.path.segments[0])
                } else {
                    scope.resolve_path(&function.path)
                };
                let path = match target {
                    EntityPath::Rust(v) => v,
                    EntityPath::Cpp(_) => {
                        ctx.add_error_str(
                            "a free function cannot be declared inside a c++:: scope",
                            f.span,
                        );
                        return;
                    }
                };
                let func = ZngurFn {
                    path: RustPathAndGenerics {
                        path,
                        generics: method.generics,
                        named_generics: vec![],
                    },
                    inputs: method.inputs,
                    output: method.output,
                    is_safe,
                };
                ctx.record_span(&func, f.span.into_range());
                checked_merge(func, &mut r.spec, f.span, ctx, None);
            }
            ProcessedItem::ExternCpp(items) => {
                for item in items {
                    match item {
                        ParsedExternCppItem::Function { is_safe, method } => {
                            let span = method.span;
                            let method = method.inner.to_zngur(scope, ctx);
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
                                    tr: tr.map(|x| x.to_zngur(scope, ctx)),
                                    ty: ty.inner.to_zngur(scope, ctx),
                                    methods: methods
                                        .into_iter()
                                        .map(|(is_safe, x)| {
                                            let mut m = x.to_zngur(scope, ctx);
                                            m.is_safe = Some(is_safe);
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
            ProcessedItem::CrateName(name, span) => {
                if !matches!(&scope.base, EntityPath::Rust(v) if v.is_empty()) {
                    ctx.add_error_str(
                        "`#crate` can only be used at the top level of a file, not inside a `mod` block",
                        span,
                    );
                    return;
                }
                if ctx.depth > 0 {
                    ctx.add_error_str(
                        "`#crate` can only be used in the top-level file of a zngur module, not in merged files",
                        span,
                    );
                    return;
                }
                if is_reserved_crate_name(name) {
                    ctx.add_error_str(&format!("`{name}` cannot be used as a `#crate` name"), span);
                    return;
                }
                match &r.spec.crate_name {
                    Some(existing) if existing != name => ctx.add_error_str(
                        &format!("conflicting `#crate`: already set to `{existing}`"),
                        span,
                    ),
                    _ => r.spec.crate_name = Some(name.to_owned()),
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
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> RustType {
        match self {
            ParsedRustType::Primitive(s) => RustType::Primitive(s),
            ParsedRustType::Ref(m, s) => RustType::Ref(m, Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Raw(m, s) => RustType::Raw(m, Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Boxed(s) => RustType::Boxed(Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Slice(s) => RustType::Slice(Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Dyn(tr, bounds) => RustType::Dyn(
                tr.to_zngur(scope, ctx),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Impl(tr, bounds) => RustType::Impl(
                tr.to_zngur(scope, ctx),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Tuple(v) => {
                RustType::Tuple(v.into_iter().map(|s| s.to_zngur(scope, ctx)).collect())
            }
            ParsedRustType::Adt(s) => match scope.as_type_var(&s) {
                Some(v) => RustType::TypeVar(v),
                None => match scope.resolve_path(&s.path) {
                    EntityPath::Cpp(segs) => {
                        if !s.generics.is_empty() || !s.named_generics.is_empty() {
                            scope.report_invalid_cpp_path(
                                ctx,
                                &s.path,
                                "`c++::` paths do not support generic arguments",
                                "paths",
                            );
                        }
                        let _ = s.resolve_generics(scope, ctx);
                        RustType::Cpp(segs)
                    }
                    EntityPath::Rust(path) => {
                        RustType::Adt(s.into_rust_path_and_generics(path, scope, ctx))
                    }
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
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> RustTrait {
        match self {
            ParsedRustTrait::Normal(s) => {
                let path = match scope.resolve_path(&s.path) {
                    EntityPath::Rust(v) => v,
                    EntityPath::Cpp(segs) => {
                        scope.report_invalid_cpp_path(
                            ctx,
                            &s.path,
                            "`c++::` paths cannot be used as a trait",
                            "traits",
                        );
                        segs
                    }
                };
                RustTrait::Normal(s.into_rust_path_and_generics(path, scope, ctx))
            }
            ParsedRustTrait::Fn {
                name,
                inputs,
                output,
            } => RustTrait::Fn {
                name: name.to_owned(),
                inputs: inputs.into_iter().map(|s| s.to_zngur(scope, ctx)).collect(),
                output: Box::new(output.to_zngur(scope, ctx)),
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
    fn resolve_generics(
        self,
        scope: &Scope<'_>,
        ctx: &mut ParseContext,
    ) -> (Vec<RustType>, Vec<(String, RustType)>) {
        let generics = self
            .generics
            .into_iter()
            .map(|x| x.to_zngur(scope, ctx))
            .collect();
        let named_generics = self
            .named_generics
            .into_iter()
            .map(|(name, x)| (name.to_owned(), x.to_zngur(scope, ctx)))
            .collect();
        (generics, named_generics)
    }

    fn into_rust_path_and_generics(
        self,
        path: Vec<String>,
        scope: &Scope<'_>,
        ctx: &mut ParseContext,
    ) -> RustPathAndGenerics {
        let (generics, named_generics) = self.resolve_generics(scope, ctx);
        RustPathAndGenerics {
            path,
            generics,
            named_generics,
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
    /// a `#trivially_relocatable` directive
    /// outer_ty
    TriviallyRelocatable(RustType),
    /// a [`ZngurWellknownTrait`](zngur_def::ZngurWellknownTrait)
    /// outer_ty, trait
    WellknownTrait(RustType, ZngurWellknownTrait),
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

/// Lexically normalizes `.` and `..` components in `path` without touching the filesystem.
pub(crate) fn normalize_lexical_path(path: &std::path::Path) -> std::path::PathBuf {
    let out: std::path::PathBuf = normalize_lexical_components(path).into_iter().collect();
    if out.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        out
    }
}

#[derive(Clone)]
struct CachedImportedModule {
    crate_name: Option<String>,
    cpp_types: Vec<Vec<String>>,
    had_errors: bool,
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
    /// Parent directory of the top-level `.zng` file of the module currently being parsed.
    /// Relative `import` paths inside nested `merge` files are rebased onto this directory.
    module_root_dir: Option<std::path::PathBuf>,
    source: &'source str,
    source_id: SourceId,
    depth: usize,
    /// Whether `import`ed modules are parsed (to find their `#crate` and `c++::` types). Only
    /// for the module being generated, not for the imported modules themselves.
    resolve_module_imports: bool,
    cfg_provider: &'cfg dyn RustCfgProvider,
    report_sink: &'cfg mut dyn ReportSink,
    /// All .zng files processed during parsing (main file + imports)
    processed_files: OwnedRefMut<'this, Vec<std::path::PathBuf>>,
    report_counter: OwnedRefMut<'this, ReportCounter>,
    sources: OwnedRefMut<'this, indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>>,
    recorded_spans: OwnedRefMut<'this, indexmap::IndexMap<SpanKey, Vec<ReportSpan>>>,
    /// Already-parsed `import`ed modules, keyed by normalized path, so multiple `merge`d files
    /// importing the same module parse and record it at most once.
    parsed_module_cache:
        OwnedRefMut<'this, std::collections::HashMap<std::path::PathBuf, CachedImportedModule>>,
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
            module_root_dir: path.parent().map(std::path::Path::to_path_buf),
            source_id,
            source,
            depth: 0,
            resolve_module_imports: true,
            cfg_provider: cfg,
            report_sink,
            processed_files,
            report_counter: Default::default(),
            sources,
            recorded_spans: Default::default(),
            parsed_module_cache: Default::default(),
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
            module_root_dir: self.module_root_dir.clone(),
            source_id,
            source,
            depth: self.depth + 1,
            resolve_module_imports: self.resolve_module_imports,
            cfg_provider: self.cfg_provider,
            report_sink: self.report_sink,
            processed_files: self.processed_files.into_borrowed(),
            report_counter: self.report_counter.into_borrowed(),
            sources: self.sources.into_borrowed(),
            recorded_spans: self.recorded_spans.into_borrowed(),
            parsed_module_cache: self.parsed_module_cache.into_borrowed(),
        }
    }

    /// Build a parse context for the top-level file of an `import`ed module, which is parsed
    /// into its own spec (it is the top level of that module with its own isolated span table,
    /// and its own `import`s aren't parsed).
    fn nested_module<'borrowed, 'src>(
        &'borrowed mut self,
        path: &'src std::path::Path,
        source: &'src str,
    ) -> ParseContext<'borrowed, 'src, 'borrowed> {
        let mut ctx = self.nested(path, source);
        ctx.module_root_dir = path.parent().map(std::path::Path::to_path_buf);
        ctx.depth = 0;
        ctx.resolve_module_imports = false;
        ctx.recorded_spans = OwnedRefMut::Owned(Default::default());
        ctx
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

    /// Resolves the best diagnostic [`ReportSpan`] for an item `key` inside `ty`, falling back
    /// to `template_locations` (if instantiated from a template), `ty`'s first declaration span,
    /// or `(0, 0..0)`.
    fn resolve_item_span(
        &self,
        key: &PartialSpanKey,
        template_locations: &[ReportSpan],
        ty: &ZngurType,
    ) -> ReportSpan {
        self.fetch_spans_global(key)
            .into_iter()
            .next_back()
            .cloned()
            .or_else(|| template_locations.last().cloned())
            .or_else(|| self.fetch_spans_global(ty).into_iter().next().cloned())
            .unwrap_or((0, 0usize..0))
    }

    /// Records a single-span fatal error [`Report`] at `span` whose message and red label match.
    fn add_fatal_error_at_span(&mut self, span: ReportSpan, msg: &str) {
        self.add_fatal_report(
            Report::build(ReportKind::Error, span.clone())
                .with_message(msg)
                .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                .finish(),
        );
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

/// Make snapshot stable for windows.
fn import_error_message(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "No such file or directory".to_owned()
    } else {
        error.to_string()
    }
}

impl ImportResolver for DefaultImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd
            .join(relpath)
            .canonicalize()
            .map_err(import_error_message)?;
        std::fs::read_to_string(path).map_err(import_error_message)
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

fn is_reserved_crate_name(name: &str) -> bool {
    matches!(
        name,
        "Self"
            | "abstract"
            | "as"
            | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}

fn normalize_lexical_components(path: &std::path::Path) -> Vec<Component<'_>> {
    let mut out = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) => {}
                _ => out.push(Component::ParentDir),
            },
            other => out.push(other),
        }
    }
    out
}

/// Rebases a `./`- or `../`-prefixed `import` path written inside a `merge`d file (`current_dir`)
/// so that it is relative to the top-level `.zng` file's directory (`module_root_dir`). Bare
/// C++ include-search paths (like `"packet.zng"`) are returned unchanged.
fn rebase_relative_import_path(
    module_root_dir: &std::path::Path,
    current_dir: &std::path::Path,
    import_path: &std::path::Path,
) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    if !matches!(
        import_path.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    ) {
        return import_path.to_path_buf();
    }
    let joined = current_dir.join(import_path);
    let root_comps = normalize_lexical_components(module_root_dir);
    let joined_comps = normalize_lexical_components(&joined);
    let common = root_comps
        .iter()
        .zip(&joined_comps)
        .take_while(|(a, b)| a == b)
        .count();
    if !root_comps[common..]
        .iter()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return joined;
    }
    let up_count = root_comps[common..].len();
    if let Some(tail_utf8) = joined_comps[common..]
        .iter()
        .map(|comp| comp.as_os_str().to_str())
        .collect::<Option<Vec<&str>>>()
    {
        let mut parts: Vec<&str> = Vec::with_capacity(up_count + tail_utf8.len());
        parts.extend(std::iter::repeat_n("..", up_count));
        parts.extend(tail_utf8);
        return if parts.is_empty() {
            PathBuf::from(".")
        } else if parts[0] == ".." {
            PathBuf::from(parts.join("/"))
        } else {
            PathBuf::from(format!("./{}", parts.join("/")))
        };
    }
    let mut out = PathBuf::new();
    for _ in 0..up_count {
        out.push("..");
    }
    for comp in &joined_comps[common..] {
        out.push(comp.as_os_str());
    }
    if matches!(out.components().next(), Some(Component::ParentDir)) {
        out
    } else {
        Path::new(".").join(out)
    }
}

impl<'a> ParsedZngFile<'a> {
    /// Parses the modules that the current file `import`s (from `first` on), to find their
    /// `#crate` and the `c++::` types they declare, which this module can then refer to.
    fn resolve_module_imports(
        zngur: &mut ZngurSpecBuilder,
        first: usize,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let Some(base_dir) = ctx
            .module_root_dir
            .clone()
            .or_else(|| ctx.path.parent().map(std::path::Path::to_path_buf))
        else {
            return;
        };
        for index in first..zngur.spec.imported_modules.len() {
            let import_path = zngur.spec.imported_modules[index].path.clone();
            let report_span = zngur.module_import_spans[index].clone();
            let path = base_dir.join(&import_path);
            let norm_path = normalize_lexical_path(&path);
            let cached = if let Some(cached) = ctx.parsed_module_cache.get(&norm_path).cloned() {
                cached
            } else {
                let Ok(text) = resolver.resolve_import(&base_dir, &import_path) else {
                    // Not an error: an `import` only has to name the generated header (found via
                    // the C++ include path), so the `.zng` file may not be at this relative path.
                    // Its `c++::` types just can't be referred to from this module then.
                    continue;
                };
                let errors_before = ctx.reports().errors;
                let imported = {
                    let mut module_ctx = ctx.nested_module(&path, &text);
                    let mut module = ZngurSpecBuilder::default();
                    Self::parse_into(&mut module, &mut module_ctx, resolver);
                    module.to_zngur(&mut module_ctx)
                };
                let had_errors = ctx.reports().errors > errors_before;
                let cpp_types: Vec<Vec<String>> = imported
                    .types
                    .iter()
                    .filter_map(|ty| match &ty.ty {
                        RustType::Cpp(segs) => Some(segs.clone()),
                        _ => None,
                    })
                    .collect();
                let cached = CachedImportedModule {
                    crate_name: imported.crate_name,
                    cpp_types,
                    had_errors,
                };
                ctx.parsed_module_cache.insert(norm_path, cached.clone());
                cached
            };
            if !cached.had_errors && !cached.cpp_types.is_empty() && cached.crate_name.is_none() {
                let msg = format!(
                    "imported module `{}` declares `c++::` types, so it needs a `#crate <name>` \
                     directive for this module to find their Rust wrappers \
                     (at `::<name>::generated::...`)",
                    import_path.display()
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
            let module_import = &mut zngur.spec.imported_modules[index];
            module_import.crate_name = cached.crate_name;
            module_import.cpp_types = cached.cpp_types;
        }
    }

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
        let errors_before = ctx.reports().errors;
        let first_new_module_import = zngur.spec.imported_modules.len();
        ProcessedZngFile::new(aliases, items).into_zngur_spec(zngur, ctx);
        if ctx.reports().errors > errors_before {
            return;
        }
        if ctx.resolve_module_imports {
            Self::resolve_module_imports(zngur, first_new_module_import, ctx, resolver);
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
        ParsedItem::CrateName(name, span) => Ret::Processed(ProcessedItem::CrateName(name, span)),
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
        ParsedItem::Fn { is_safe, function } => {
            Ret::Processed(ProcessedItem::Fn { is_safe, function })
        }
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
        let root_scope = Scope::new_root(self.aliases, ctx);

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
    /// The span of each entry of `spec.imported_modules`, in the file that imports it.
    module_import_spans: Vec<ReportSpan>,
}

impl ZngurSpecBuilder {
    fn to_zngur(self, ctx: &mut ParseContext) -> ZngurSpec {
        let ZngurSpecBuilder {
            mut spec,
            templates,
            imports: _,
            module_import_spans,
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

            let field_spans = ty
                .fields
                .iter()
                .filter_map(|field| {
                    let span = ctx
                        .fetch_spans_global(&field.into_span_key_with_variant(ty.ty.clone(), None))
                        .into_iter()
                        .next()
                        .cloned()
                        .or_else(|| ctx.fetch_spans_global(ty).into_iter().next().cloned())?;
                    Some((cpp_handle_field_name(&field.name), span))
                })
                .collect::<HashMap<_, _>>();
            let mut reported_names = HashSet::new();
            for method in &ty.methods {
                let name =
                    cpp_handle_keyword(method.cpp_name.as_deref().unwrap_or(&method.data.name));
                let Some(field_span) = field_spans.get(&name) else {
                    continue;
                };
                if !reported_names.insert(name.clone()) {
                    continue;
                }
                let method_span = ctx
                    .fetch_spans_global(&method.data.into_span_key_with(ty.ty.clone()))
                    .into_iter()
                    .next()
                    .cloned()
                    .or_else(|| ctx.fetch_spans_global(ty).into_iter().next().cloned());
                let Some(method_span) = method_span else {
                    continue;
                };
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, method_span.clone())
                        .with_message(format!(
                            "C++ field and method both use the name `{name}` on `{}`.",
                            ty.ty
                        ))
                        .with_label(
                            Label::new((*field_span).clone())
                                .with_message("field is exposed with this C++ name")
                                .with_color(Color::Yellow),
                        )
                        .with_label(
                            Label::new(method_span)
                                .with_message("method is exposed with this C++ name")
                                .with_color(Color::Red),
                        )
                        .with_note(
                            "Rename the field or give the method a different C++ name with `as`.",
                        )
                        .finish(),
                );
            }

            if !ty.wellknown_traits.iter().any(|wkt| {
                matches!(
                    wkt,
                    ZngurWellknownTrait::Copy | ZngurWellknownTrait::Unsized
                )
            }) {
                ty.wellknown_traits.push(ZngurWellknownTrait::Drop);
            }
            let mut cpp_directives: Vec<(&str, PartialSpanKey)> = Vec::new();
            if ty.cpp_stack_owned.is_some() {
                cpp_directives.push((
                    "#cpp_stack_owned",
                    PartialSpanKey::CppStackOwned(ty.ty.clone()),
                ));
            }
            if ty.cpp_ref.is_some() {
                cpp_directives.push(("#cpp_ref", PartialSpanKey::CppRef(ty.ty.clone())));
            }
            if ty.cpp_heap_allocated.is_some() {
                cpp_directives.push((
                    "#cpp_heap_allocated",
                    PartialSpanKey::CppHeapAllocated(ty.ty.clone()),
                ));
            }
            if cpp_directives.len() > 1 {
                let resolved: Vec<_> = cpp_directives
                    .iter()
                    .map(|(name, key)| {
                        (*name, ctx.fetch_spans_global(key).last().copied().cloned())
                    })
                    .collect();
                let (first_name, ref first_span) = resolved[0];
                let (second_name, ref second_span) = resolved[1];
                let report_span = second_span
                    .clone()
                    .or_else(|| first_span.clone())
                    .unwrap_or((0, 0usize..0));
                let mut report =
                    Report::build(ReportKind::Error, report_span).with_message(format!(
                        "`{first_name}` cannot be used with `{second_name}` for type {}.",
                        ty.ty
                    ));
                for (dir_name, span) in resolved {
                    if let Some(span) = span {
                        report = report.with_label(
                            Label::new(span)
                                .with_message(format!("`{dir_name}` declared here"))
                                .with_color(Color::Yellow),
                        );
                    }
                }
                ctx.add_fatal_report(report.finish());
                continue;
            }
            if ty.wellknown_traits.contains(&ZngurWellknownTrait::Unsized)
                && (matches!(ty.ty, RustType::Cpp(_)) || !cpp_directives.is_empty())
            {
                let report_span = ctx.resolve_item_span(
                    &PartialSpanKey::WellknownTrait(ty.ty.clone(), ZngurWellknownTrait::Unsized),
                    &template_locations,
                    ty,
                );
                ctx.add_fatal_error_at_span(
                    report_span,
                    "`?Sized` is not supported on C++ types; a `c++::` type without a \
                     layout is already reference-only",
                );
                continue;
            }
            let needs_layout = !matches!(ty.ty, RustType::Cpp(_))
                || ty.cpp_stack_owned.is_some()
                || ty.cpp_heap_allocated.is_some();
            if ty.layout.is_none() && needs_layout {
                let spans = ctx.fetch_spans_global(ty);
                let (first, rest) = {
                    let mut it = spans.into_iter();
                    let first = it.next().cloned();
                    (first, it.collect::<Vec<_>>())
                };
                let mut report = Report::build(ReportKind::Error, (0, 0usize..0))
                    .with_message(format!("No layout policy found for type {}.", ty.ty));
                if ty.cpp_stack_owned.is_some() {
                    report = report.with_note(
                        "Type is declared with `#cpp_stack_owned` which requires `#layout(size = X, align = Y)`.",
                    );
                } else if ty.cpp_heap_allocated.is_some() && matches!(ty.ty, RustType::Cpp(_)) {
                    report = report.with_note(
                        "Type is declared with `#cpp_heap_allocated` which requires `#layout(size = 16, align = 8)` or `#heap_allocated`.",
                    );
                } else {
                    report = report.with_note(
                        "Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.",
                    );
                }

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
                for (i, location) in template_locations.iter().cloned().enumerate() {
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
            if ty.cpp_stack_owned.is_some()
                && matches!(
                    ty.layout,
                    Some(
                        LayoutPolicy::HeapAllocated
                            | LayoutPolicy::OnlyByRef
                            | LayoutPolicy::Conservative { .. }
                    )
                )
            {
                let stack_owned_spans =
                    ctx.fetch_spans_global(&PartialSpanKey::CppStackOwned(ty.ty.clone()));
                let layout_spans = ctx.fetch_spans_global(&PartialSpanKey::Layout(ty.ty.clone()));
                let report_span = stack_owned_spans
                    .last()
                    .copied()
                    .cloned()
                    .or_else(|| layout_spans.last().copied().cloned())
                    .unwrap_or((0, 0usize..0));

                let layout_name = match ty.layout {
                    Some(LayoutPolicy::HeapAllocated) => "#heap_allocated",
                    Some(LayoutPolicy::OnlyByRef) => "#only_by_ref",
                    Some(LayoutPolicy::Conservative { .. }) => "#layout_conservative",
                    _ => unreachable!(),
                };

                let mut report = Report::build(ReportKind::Error, report_span)
                    .with_message(format!(
                        "`#cpp_stack_owned` cannot be used with `{layout_name}` for type {}.",
                        ty.ty
                    ))
                    .with_note("`#cpp_stack_owned` types must use `#layout(size = X, align = Y)`.");

                if let Some(span) = stack_owned_spans.last() {
                    report = report.with_label(
                        Label::new((*span).clone())
                            .with_message("`#cpp_stack_owned` declared here")
                            .with_color(Color::Yellow),
                    );
                }
                if let Some(span) = layout_spans.last() {
                    report = report.with_label(
                        Label::new((*span).clone())
                            .with_message(format!("`{layout_name}` declared here"))
                            .with_color(Color::Yellow),
                    );
                }

                ctx.add_fatal_report(report.finish());
            } else if matches!(ty.ty, RustType::Cpp(_))
                && matches!(ty.layout, Some(LayoutPolicy::Conservative { .. }))
            {
                let report_span = ctx.resolve_item_span(
                    &PartialSpanKey::Layout(ty.ty.clone()),
                    &template_locations,
                    ty,
                );
                ctx.add_fatal_error_at_span(
                    report_span,
                    "`#layout_conservative` cannot be used on `c++::` types; use `#layout(size = X, align = Y)` instead",
                );
            } else if matches!(ty.ty, RustType::Cpp(_))
                && (matches!(ty.layout, Some(LayoutPolicy::OnlyByRef))
                    || (matches!(ty.layout, Some(LayoutPolicy::HeapAllocated))
                        && ty.cpp_heap_allocated.is_none()))
            {
                let report_span = ctx.resolve_item_span(
                    &PartialSpanKey::Layout(ty.ty.clone()),
                    &template_locations,
                    ty,
                );
                ctx.add_fatal_error_at_span(
                    report_span,
                    "`#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` \
                     type without a layout is already reference-only",
                );
            }
            if ty.trivially_relocatable {
                let tr_span = ctx.resolve_item_span(
                    &PartialSpanKey::TriviallyRelocatable(ty.ty.clone()),
                    &template_locations,
                    ty,
                );
                if !matches!(ty.ty, RustType::Cpp(_)) {
                    ctx.add_fatal_error_at_span(
                        tr_span,
                        "`#trivially_relocatable` is only for `c++::` types (Rust types \
                         are always trivially relocatable)",
                    );
                } else if !cpp_directives.is_empty() {
                    let msg = "`#trivially_relocatable` cannot be combined with \
                               `#cpp_stack_owned`, `#cpp_ref`, or `#cpp_heap_allocated`";
                    let mut report = Report::build(ReportKind::Error, tr_span.clone())
                        .with_message(msg)
                        .with_label(
                            Label::new(tr_span)
                                .with_message("`#trivially_relocatable` declared here")
                                .with_color(Color::Red),
                        );
                    for (dir_name, dir_key) in &cpp_directives {
                        if let Some(span) = ctx.fetch_spans_global(dir_key).into_iter().next_back()
                        {
                            report = report.with_label(
                                Label::new(span.clone())
                                    .with_message(format!("`{dir_name}` declared here"))
                                    .with_color(Color::Yellow),
                            );
                        }
                    }
                    ctx.add_fatal_report(report.finish());
                } else if ty.cpp_by_value_layout().is_none() {
                    ctx.add_fatal_error_at_span(
                        tr_span,
                        "`#trivially_relocatable` needs `#layout(size = X, align = Y)`, \
                         which Rust needs to hold the C++ type by value",
                    );
                }
            }
        }
        // Check that imported `c++::` types do not collide with local `c++::` declarations or
        // with `c++::` types imported from a different crate.
        let mut seen_reexports: HashSet<(String, Vec<String>)> = HashSet::new();
        let mut imported_owners: std::collections::HashMap<Vec<String>, (String, ReportSpan)> =
            std::collections::HashMap::new();
        for (module, import_span) in spec.imported_modules.iter().zip(&module_import_spans) {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let norm_crate = crate_name.replace('-', "_");
            if let Some(own_crate) = &spec.crate_name
                && own_crate.replace('-', "_") == norm_crate
            {
                let msg = format!(
                    "imported module `{}` has the same `#crate` (`{crate_name}`) as this module; \
                     use `merge \"{}\";` to split `.zng` files within a single crate",
                    module.path.display(),
                    module.path.display(),
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, import_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(import_span.clone())
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                continue;
            }
            for segs in &module.cpp_types {
                if !seen_reexports.insert((norm_crate.clone(), segs.clone())) {
                    continue;
                }
                let local_ty = RustType::Cpp(segs.clone());
                if spec.types.iter().any(|t| t.ty == local_ty) {
                    let local_span = ctx
                        .fetch_spans_global(&local_ty)
                        .first()
                        .copied()
                        .cloned()
                        .unwrap_or_else(|| import_span.clone());
                    let msg = format!(
                        "`c++::{}` is already imported from `{}` (`#crate {crate_name}`), and \
                         cannot also be declared in this module",
                        segs.join("::"),
                        module.path.display(),
                    );
                    let report = Report::build(ReportKind::Error, local_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(local_span)
                                .with_message("declared locally here")
                                .with_color(Color::Red),
                        )
                        .with_label(
                            Label::new(import_span.clone())
                                .with_message(format!(
                                    "imported from `{}` (`#crate {crate_name}`) here",
                                    module.path.display()
                                ))
                                .with_color(Color::Yellow),
                        )
                        .finish();
                    ctx.add_fatal_report(report);
                } else if let Some((prev_crate, prev_span)) = imported_owners.get(segs) {
                    let msg = format!(
                        "`c++::{}` is imported from multiple crates (`{prev_crate}` and \
                         `{norm_crate}`)",
                        segs.join("::")
                    );
                    let report = Report::build(ReportKind::Error, import_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(import_span.clone())
                                .with_message(format!("also imported from `{norm_crate}` here"))
                                .with_color(Color::Red),
                        )
                        .with_label(
                            Label::new(prev_span.clone())
                                .with_message(format!("first imported from `{prev_crate}` here"))
                                .with_color(Color::Yellow),
                        )
                        .finish();
                    ctx.add_fatal_report(report);
                } else {
                    imported_owners.insert(segs.clone(), (norm_crate.clone(), import_span.clone()));
                }
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

    fn as_word(&self) -> Option<&'a str> {
        match self {
            Token::Ident(x) => Some(x),
            Token::KwAs => Some("as"),
            Token::KwAsync => Some("async"),
            Token::KwDyn => Some("dyn"),
            Token::KwUse => Some("use"),
            Token::KwFor => Some("for"),
            Token::KwMod => Some("mod"),
            Token::KwCrate => Some("crate"),
            Token::KwType => Some("type"),
            Token::KwTrait => Some("trait"),
            Token::KwFn => Some("fn"),
            Token::KwMut => Some("mut"),
            Token::KwConst => Some("const"),
            Token::KwExtern => Some("extern"),
            Token::KwImpl => Some("impl"),
            Token::KwImport => Some("import"),
            Token::KwMerge => Some("merge"),
            Token::KwIf => Some("if"),
            Token::KwElse => Some("else"),
            Token::KwMatch => Some("match"),
            Token::KwSafe => Some("safe"),
            Token::KwUnsafe => Some("unsafe"),
            Token::Arrow
            | Token::ArrowArm
            | Token::AngleOpen
            | Token::AngleClose
            | Token::BracketOpen
            | Token::BracketClose
            | Token::Colon
            | Token::ColonColon
            | Token::ParenOpen
            | Token::ParenClose
            | Token::BraceOpen
            | Token::BraceClose
            | Token::And
            | Token::Star
            | Token::Sharp
            | Token::Plus
            | Token::Eq
            | Token::Question
            | Token::Comma
            | Token::Semicolon
            | Token::Pipe
            | Token::Underscore
            | Token::Dot
            | Token::Bang
            | Token::CppPathStart
            | Token::Str(_)
            | Token::RawStr(_, _)
            | Token::Number(_) => None,
        }
    }
}

impl Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(word) = self.as_word() {
            return write!(f, "{word}");
        }
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
            Token::CppPathStart => write!(f, "c++"),
            Token::Number(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, r#""{s}""#),
            Token::RawStr(hashes, s) => {
                let h = "#".repeat(*hashes);
                write!(f, r#"r{h}"{s}"{h}"#)
            }
            Token::Ident(_)
            | Token::KwAs
            | Token::KwAsync
            | Token::KwDyn
            | Token::KwUse
            | Token::KwFor
            | Token::KwMod
            | Token::KwCrate
            | Token::KwType
            | Token::KwTrait
            | Token::KwFn
            | Token::KwMut
            | Token::KwConst
            | Token::KwExtern
            | Token::KwImpl
            | Token::KwImport
            | Token::KwMerge
            | Token::KwIf
            | Token::KwElse
            | Token::KwMatch
            | Token::KwSafe
            | Token::KwUnsafe => unreachable!(),
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
        raw_string,
        plain_string,
        just("c++").to(Token::CppPathStart),
        text::ident()
            .filter(|ident: &&str| *ident != "_")
            .map(Token::ident_or_kw),
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

        let trait_bound = just(Token::Plus)
            .ignore_then(path())
            .try_map_with(|path, extra| {
                let qualified = path.segments.len() > 1 || path.start != ParsedPathStart::Relative;
                let valid_qualified_path = matches!(
                    (path.start, path.segments.as_slice()),
                    (
                        ParsedPathStart::Relative | ParsedPathStart::Absolute,
                        ["std" | "core", "marker", _]
                    )
                );
                if qualified && !valid_qualified_path {
                    return Err(Rich::custom(
                        extra.span(),
                        "qualified trait bounds must use `std::marker` or `core::marker`",
                    ));
                }
                path.segments
                    .last()
                    .copied()
                    .ok_or_else(|| Rich::custom(extra.span(), "expected a trait bound path"))
            });

        let dyn_trait = just(Token::KwDyn)
            .or(just(Token::KwImpl))
            .then(rust_trait(parser.clone()))
            .then(trait_bound.repeated().collect::<Vec<_>>().boxed())
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
    choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ))
    .or_not()
    .then(method_signature())
    .map(|(is_safe, mut method)| {
        method.is_safe = is_safe;
        method
    })
    .boxed()
}

fn method_signature<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedMethod<'a>, ZngParserExtra<'a>> + Clone {
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
            parsed_method_from_signature(name, opt_async.map(|x| x.span), generics, args)
        })
        .boxed()
}

fn function_signature<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedFunction<'a>, ZngParserExtra<'a>> + Clone {
    let function_path = path().try_map_with(|path, extra| {
        if path.segments.is_empty() {
            Err(Rich::custom(extra.span(), "expected function name"))
        } else {
            Ok(path)
        }
    });
    spanned(just(Token::KwAsync))
        .or_not()
        .then_ignore(just(Token::KwFn))
        .then(function_path)
        .then(
            rust_type()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .or(empty().to(vec![]))
                .boxed(),
        )
        .then(fn_args(rust_type()))
        .map(|(((opt_async, path), generics), args)| {
            let name = *path
                .segments
                .last()
                .expect("function paths must contain a name");
            ParsedFunction {
                path,
                signature: parsed_method_from_signature(
                    name,
                    opt_async.map(|x| x.span),
                    generics,
                    args,
                ),
            }
        })
        .boxed()
}

fn parsed_method_from_signature<'a>(
    name: &'a str,
    async_span: Option<Span>,
    generics: Vec<ParsedRustType<'a>>,
    args: (Vec<ParsedRustType<'a>>, ParsedRustType<'a>),
) -> ParsedMethod<'a> {
    let is_self = |ty: &ParsedRustType<'_>| {
        if let ParsedRustType::Adt(ty) = ty {
            ty.path.start == ParsedPathStart::Relative
                && &ty.path.segments == &["self"]
                && ty.generics.is_empty()
        } else {
            false
        }
    };
    let (inputs, receiver) = match args.0.first() {
        Some(ty) if is_self(ty) => (args.0[1..].to_vec(), ZngurMethodReceiver::Move),
        Some(ParsedRustType::Ref(mutability, ty)) if is_self(ty) => {
            (args.0[1..].to_vec(), ZngurMethodReceiver::Ref(*mutability))
        }
        _ => (args.0, ZngurMethodReceiver::Static),
    };
    let mut output = args.1;
    if let Some(async_span) = async_span {
        output = ParsedRustType::Impl(
            ParsedRustTrait::Normal(ParsedRustPathAndGenerics {
                path: ParsedPath {
                    start: ParsedPathStart::Absolute,
                    segments: vec!["std", "future", "Future"],
                    span: async_span,
                },
                generics: vec![],
                named_generics: vec![("Output", output)],
            }),
            vec![],
        )
    }
    ParsedMethod {
        name,
        is_safe: None,
        receiver,
        generics,
        inputs,
        output,
    }
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
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .map(|cpp_type| ParsedTypeItem::CppHeapAllocated { cpp_type });
    let cpp_ref = just(Token::Sharp)
        .then(just(Token::Ident("cpp_ref")))
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .map(|x| ParsedTypeItem::CppRef { cpp_type: x });
    let cpp_stack_owned = just(Token::Sharp)
        .then(just(Token::Ident("cpp_stack_owned")))
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .then(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .or_not()
                .boxed(),
        )
        .map(|(cpp_type, props)| ParsedTypeItem::CppStackOwned { cpp_type, props });
    let trivially_relocatable = just([Token::Sharp, Token::Ident("trivially_relocatable")])
        .to(ParsedTypeItem::TriviallyRelocatable);

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
            trivially_relocatable.boxed(),
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
    choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ))
    .or_not()
    .then(spanned(function_signature()))
    .then_ignore(just(Token::Semicolon))
    .map(|(is_safe, function)| ParsedItem::Fn { is_safe, function })
    .boxed()
}

fn word_token<'a>() -> impl Parser<'a, ParserInput<'a>, &'a str, ZngParserExtra<'a>> + Clone {
    select! {
        tok if Token::as_word(&tok).is_some() => Token::as_word(&tok).unwrap(),
    }
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
            just(Token::KwCrate)
                .ignore_then(word_token())
                .map_with(|name, extra| ParsedItem::CrateName(name, extra.span()))
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
        .then(spanned(method_signature()))
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
                .then(method_signature())
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
