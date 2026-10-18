use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt::Display,
    ops::Deref,
    rc::Rc,
};

use crate::{
    attrs::{
        attributes::{
            emit::{EmitOnImplParser, EmitParser},
            emit_items::EmitItemsParser,
            layout::LayoutParser,
            policy::PolicyParser,
            rename::RenameParser,
        },
        parse::{PartialParsedAttr, parser::ParserBuilder},
    },
    ctx::AutozngContext,
    driver::with_ctxt,
    metadata::IntoMetadataKey,
};

// implementation of attrs
mod macros;
mod parse;

// attrs for ast/mir node types
mod attrs_for;
pub(crate) use attrs_for::AutozngAttrsExt;

// attr implementations
pub mod attributes;

// impl of attrs on ast/mir node types
pub mod impls;

/// namespace of autozng attrs
const AUTOZNG_NAMESPACE: &str = "autozng";
const PATH_SEP: &str = "::";

type DynAttrBuilder = dyn AttrBuilder + Sync;

static ATTR_BUILDERS: &[&DynAttrBuilder] = &[
    &ParserBuilder::<EmitParser>::new(),
    &ParserBuilder::<EmitOnImplParser>::new(),
    &ParserBuilder::<EmitItemsParser>::new(),
    &ParserBuilder::<RenameParser>::new(),
    &ParserBuilder::<PolicyParser>::new(),
    &ParserBuilder::<LayoutParser>::new(),
];

/// given a partially parsed attr select a builder by the attr's path
pub fn parse_with_builder_for_attr(
    attr: PartialParsedAttr,
    item: &dyn AttrTarget,
) -> Result<Option<Rc<dyn DynAttr>>, ()> {
    if let Some(builder) = ATTR_BUILDERS
        .iter()
        .find(|builder| builder.valid_for(&attr.path))
    {
        Ok(builder.parse(attr, item))
    } else {
        Err(())
    }
}

/// Attribute builder, should be constructed statically
pub trait AttrBuilder: std::fmt::Debug {
    /// the attribute path to construct this attribute for
    fn valid_for(&self, path: &AttrPath) -> bool;

    // Kept for the parser framework's full API; not every builder needs its paths listed yet.
    #[allow(dead_code)]
    fn paths(&self) -> Vec<AttrPath>;

    /// parse the attr into a full object or error
    ///
    /// needs a static self to preserve reference to the builder
    fn parse(&self, attr: PartialParsedAttr, item: &dyn AttrTarget) -> Option<Rc<dyn DynAttr>>;
}

#[derive(Clone)]
pub enum AttrPath {
    Static(&'static [&'static str]),
    Owned(Box<[String]>),
}

impl AttrPath {
    pub const fn from_static(value: &'static [&'static str]) -> Self {
        AttrPath::Static(value)
    }

    pub fn from<T: Into<AttrPath>>(value: T) -> Self {
        value.into()
    }

    #[allow(dead_code)] // part of the framework's path API; callers iterate `as_ident`/`Display`
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        enum AttrPathIter<'a, S: Iterator<Item = &'a str>, U: Iterator<Item = &'a str>> {
            Static(S),
            Owned(U),
        }
        impl<'a, S: Iterator<Item = &'a str>, U: Iterator<Item = &'a str>> Iterator
            for AttrPathIter<'a, S, U>
        {
            type Item = &'a str;
            fn next(&mut self) -> Option<Self::Item> {
                match self {
                    Self::Static(s) => s.next(),
                    Self::Owned(o) => o.next(),
                }
            }
        }
        match self {
            Self::Static(s) => AttrPathIter::Static(s.iter().copied()),
            Self::Owned(o) => AttrPathIter::Owned(o.as_ref().iter().map(String::as_str)),
        }
    }

    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Self::Static([ident]) => Some(ident),
            Self::Owned(o) if let [ident] = o.as_ref() => Some(ident.as_str()),
            _ => None,
        }
    }
}

impl std::cmp::PartialEq for AttrPath {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (AttrPath::Static(this), AttrPath::Static(other)) => this == other,
            (AttrPath::Static(this), AttrPath::Owned(other)) => **this == **other,
            (AttrPath::Owned(this), AttrPath::Static(other)) => **this == **other,
            (AttrPath::Owned(this), AttrPath::Owned(other)) => this == other,
        }
    }
}

impl std::cmp::Eq for AttrPath {}

/// impl hash such that a owned and static sequence are identical
impl std::hash::Hash for AttrPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let segments_iter: &mut dyn Iterator<Item = &str> = match self {
            AttrPath::Static(this) => &mut this.iter().copied(),
            AttrPath::Owned(this) => &mut this.deref().iter().map(AsRef::as_ref),
        };
        for segment in segments_iter {
            segment.hash(state);
        }
    }
}

impl std::fmt::Display for AttrPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let it: &mut dyn Iterator<Item = &str> = match self {
            AttrPath::Static(this) => &mut this.iter().copied(),
            AttrPath::Owned(this) => &mut this.deref().iter().map(AsRef::as_ref),
        };

        if let Some(sec) = it.next() {
            f.write_str(sec)?;
        }
        for sec in it {
            f.write_str(PATH_SEP)?;
            f.write_str(sec)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for AttrPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl core::cmp::PartialEq<&[&str]> for AttrPath {
    fn eq(&self, other: &&[&str]) -> bool {
        match self {
            AttrPath::Owned(this) => **this == **other,
            AttrPath::Static(this) => this == other,
        }
    }
}

impl core::cmp::PartialEq<&[String]> for AttrPath {
    fn eq(&self, other: &&[String]) -> bool {
        match self {
            AttrPath::Owned(this) => **this == **other,
            AttrPath::Static(this) => this == other,
        }
    }
}

impl<T> From<T> for AttrPath
where
    T: IntoIterator,
    T::Item: Display,
{
    fn from(value: T) -> Self {
        value.into_iter().collect()
    }
}

impl<I> FromIterator<I> for AttrPath
where
    I: Display,
{
    fn from_iter<T: IntoIterator<Item = I>>(iter: T) -> Self {
        AttrPath::Owned(iter.into_iter().map(|s| s.to_string()).collect())
    }
}

/// an Autozng Attribute
///
/// Attributes must be `Clone`: a parent's child-targeted attribute is forked onto each child
/// (see [`Attr::for_child`]) and then merged with the child's own attributes, which requires
/// cloning the derived [`DynAttr`] behind a fresh, uniquely-owned `Rc` (see
/// [`DynAttr::clone_dyn`]).
pub trait Attr: std::fmt::Debug + Clone + Sized {
    type Target: ?Sized;

    /// the path the attr is constructed form
    fn path(&self) -> AttrPath;

    /// build an `dyn Attr` if this attribute is applied to a child item
    fn for_child(&self, item: &Self::Target) -> Option<Self> {
        let _ = item;
        None
    }

    /// Merge other attribute with the same builder path into self
    /// Should only be called with self and an other of the same underlying type,
    /// so long as two attribute types never share a path this should be a simple invariant to fulfill.
    ///
    /// default implementation discards other to preserve self
    fn merge(&mut self, other: Self) {
        let _ = other;
    }

    /// visit an `Attributable` item with this attr
    fn visit(&self, item: &Self::Target, ctx: &AutozngContext) {
        let _ = ctx;
        let _ = item;
    }
}

pub trait DynAttr: Any {
    /// the path the attr is constructed form
    fn path(&self) -> AttrPath;

    /// Clone this attribute behind a new, uniquely-owned `Rc`.
    ///
    /// Used when merging a forked parent attribute with the child's own attribute: [`merge`]
    /// consumes a uniquely-owned `Rc`, but attributes cached by [`AttrsFor`] are shared.
    ///
    /// [`merge`]: DynAttr::merge
    /// [`AttrsFor`]: crate::attrs::attrs_for::AttrsFor
    fn clone_dyn(&self) -> Rc<dyn DynAttr>;

    /// build an `dyn DynAttr` if this attribute is applied to a child item
    fn for_child(&self, item: &dyn AttrTarget) -> Option<Rc<dyn DynAttr>>;

    fn merge(&mut self, other: Rc<dyn DynAttr>);

    /// visit an `Attributable` item with this attr
    fn visit(&self, item: &dyn AttrTarget, ctx: &AutozngContext);

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

impl<T: Attr + Any> DynAttr for T {
    /// the path the attr is constructed form
    fn path(&self) -> AttrPath {
        <Self as Attr>::path(self)
    }

    fn clone_dyn(&self) -> Rc<dyn DynAttr> {
        Rc::new(self.clone())
    }

    /// build an `dyn Attr` if this attribute is applied to a child item
    fn for_child(&self, item: &dyn AttrTarget) -> Option<Rc<dyn DynAttr>> {
        item.cast_ref::<<Self as Attr>::Target>().and_then(|item| {
            <Self as Attr>::for_child(self, item).map(|attr| Rc::new(attr) as Rc<dyn DynAttr>)
        })
    }

    fn merge(&mut self, other: Rc<dyn DynAttr>) {
        let path = other.path();
        if let Ok(other) = (other as Rc<dyn Any>).downcast::<T>()
            && let Ok(other) = Rc::try_unwrap(other)
        {
            <Self as Attr>::merge(self, other);
        } else {
            with_ctxt(|ctx| {
                ctx.tcx
                    .dcx()
                    .struct_err(format!("BUG! failed to unwrap duplicate attr for `{path}`"))
                    .emit();
            })
        }
    }

    /// visit an `Attributable` item with this attr
    fn visit(&self, item: &dyn AttrTarget, ctx: &AutozngContext) {
        if let Some(item) = item.cast_ref::<<Self as Attr>::Target>() {
            <Self as Attr>::visit(self, item, ctx);
        }
    }
    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as std::fmt::Debug>::fmt(self, f)
    }
}

impl std::fmt::Debug for dyn DynAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.debug_fmt(f)
    }
}

/// a fn pointer to cast a from a `&dyn DynAttrTarget` to a `&T`
/// by going through the underlying concrete type
/// caller must ensure the object is the correct underlying type
type CastAttrTargetRefFn<T> = unsafe fn(from: &dyn AttrTarget) -> &T;

#[derive(Clone)]
/// Cast impl that castes from &dyn ArrVisitable to another type `T`
pub struct AttrTargetCaster<T: ?Sized> {
    /// SAFETY: caller ensures that the `from` ref is actually a concrete type castable to `T`
    /// and the reference is valid for any contained lifetimes in the concrete type
    pub cast_ref: CastAttrTargetRefFn<T>,
}

/// a casting vtable
pub struct AttrTargetCasterTable {
    /// a map of TypeId::of::<T> to Caster<T>
    casters: &'static [(TypeId, &'static dyn Any)],
    /// TypeId of the underlying concrete type these casters are for
    type_id: TypeId,
}

impl AttrTargetCasterTable {
    fn find<T: ?Sized + 'static>(&self) -> Option<CastAttrTargetRefFn<T>> {
        self.casters.iter().find_map(|(k, v)| {
            if *k == TypeId::of::<T>() {
                v.downcast_ref::<AttrTargetCaster<T>>()
                    .map(|caster| caster.cast_ref)
            } else {
                None
            }
        })
    }
}

/// A Type that can have autozng attrs, either implicitly (emit) or explicitly
pub trait AttrTarget: IntoMetadataKey + std::fmt::Debug {
    /// a casting table for the `dyn Trait`s the underlying type implement
    /// should not be called directly use `<dyn AttrVisitable>::cast_ref` instead
    fn caster(&self) -> AttrTargetCasterTable;
    fn type_id(&self) -> TypeId;
}

impl<'a> dyn AttrTarget + 'a {
    /// cast a `&dyn AttrVisitable` to another `dyn Trait` if implemented by the underlying type using the implemented casting table
    fn cast_ref<T: ?Sized + 'static>(&self) -> Option<&T> {
        let caster = self.caster();
        debug_assert!(self.type_id() == caster.type_id);
        caster.find::<T>().map(|cast_fn| {
            // SAFETY: Caster.cast_ref called with same underlying type that created the mapping
            unsafe { cast_fn(self) }
        })
    }
}

#[allow(dead_code)] // framework helper kept alongside the casting table
trait TryFromDynAttrTarget {
    fn try_from_dyn_attr_target(value: &dyn AttrTarget) -> Option<&Self>;
}

mod diagnostics {

    use rustc_errors::into_diag_arg_using_display;
    use rustc_macros::Diagnostic;
    use rustc_span::Span;

    use crate::attrs::AttrPath;

    into_diag_arg_using_display!(AttrPath);

    #[derive(Diagnostic)]
    #[diag("unknown autozng attribute `{$path}`")]
    pub struct UnknownAutozngAttribute {
        #[primary_span]
        pub span: Span,
        pub path: AttrPath,
    }
}

/// process attrs for a given item into `dyn Attr`
pub fn process_attrs(attrs: Vec<PartialParsedAttr>, item: &dyn AttrTarget) -> Vec<Rc<dyn DynAttr>> {
    let mut results = Vec::new();

    // collect implicit emit attr
    let implicit_emit_item = item
        .cast_ref::<<attributes::emit::Emit as Attr>::Target>()
        .and_then(|item| item.implicit_emit_attr());

    // chain implicit emit attr with processed real attrs
    let attrs = implicit_emit_item
        .into_iter()
        .chain(attrs.into_iter().filter_map(|attr| {
            let span = attr.span;
            let path = attr.path.clone();
            match parse_with_builder_for_attr(attr, item) {
                Ok(result) => result,
                Err(()) => {
                    // unable to find builder
                    with_ctxt(|ctx| {
                        ctx.tcx
                            .dcx()
                            .emit_err(diagnostics::UnknownAutozngAttribute { span, path });
                    });
                    None
                }
            }
        }))
        .collect::<Vec<_>>();
    let mut unique: HashMap<AttrPath, Rc<dyn DynAttr>> = HashMap::new();

    // deduplicate and merge attrs
    for attr in attrs {
        let path = attr.path();
        match unique.entry(path) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let a = entry.get_mut();
                // merge attr with duplicate path into previous. default behavior just discards but is trait controlled.
                // SAFETY: the arc has not been cloned at this point, it was just created above
                Rc::get_mut(a).unwrap().merge(attr);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                // first attr of this path
                entry.insert(attr);
            }
        }
    }

    // put unique attrs into results
    results.extend(unique.into_values());

    results
}

// =======================================================
//  Blanket Impls
// =======================================================

pub trait AttrCollectionExt<'collection> {
    fn for_child(self, item: &dyn AttrTarget) -> Vec<Rc<dyn DynAttr>>;
    #[allow(dead_code)] // kept for inspecting an item's attribute paths (e.g. `emit_on_implementers`)
    fn get_paths(self) -> Vec<AttrPath>;
    fn visit(self, item: &dyn AttrTarget, ctx: &AutozngContext);
}

impl<'a, T: IntoIterator<Item = &'a Rc<dyn DynAttr>>> AttrCollectionExt<'a> for T {
    fn for_child(self, item: &dyn AttrTarget) -> Vec<Rc<dyn DynAttr>> {
        self.into_iter()
            .filter_map(|attr| attr.for_child(item))
            .collect()
    }

    fn get_paths(self) -> Vec<AttrPath> {
        self.into_iter().map(|attr| attr.path()).collect()
    }

    fn visit(self, item: &dyn AttrTarget, ctx: &AutozngContext) {
        for attr in self {
            attr.visit(item, ctx);
        }
    }
}
