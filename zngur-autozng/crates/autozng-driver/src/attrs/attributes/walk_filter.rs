//! `#[autozng::walk_filter(...)]` — select which interface items a recursive walk may visit.
//!
//! The attribute is accepted on the crate root (inner attribute, the default for every walk)
//! and on emit-able items: ADTs, fields, associated items, traits, and free functions. A nested
//! filter replaces the inherited filter for its whole subtree. An explicit `emit` /
//! `emit_items` overrides the filter for the selected subtree (see the walk implementation).
//!
//! The attribute accepts an ordered list of roots:
//!
//! ```ignore
//! #![autozng::walk_filter(exclude(TraitMethods))]
//! #[autozng::walk_filter(include(Fields), exclude(Named("private")))]
//! #[autozng::walk_filter(include(Fields, InherentMethods))]
//! ```
//!
//! Each matching root sets the verdict (`all` / `include` allow, `none` / `exclude` deny) and
//! the last matching root wins. With no match, the default is the inverse of the first root:
//! `include` / `none` deny, `exclude` / `all` allow.

use rustc_attr_parsing::template;
use rustc_span::Span;

use crate::attrs::parse::context::AcceptContext;
use crate::attrs::parse::parser::{
    ArgParser, AttributeParser, MetaItemOrLitParser, MetaItemParser,
};
use crate::attrs::{AUTOZNG_NAMESPACE, Attr, AttrPath};
use crate::ctx::AutozngContext;
use crate::walk::{ReceiverKind, VisibilityKind, WalkFilter, WalkFilterRoot, WalkRule};

/// An item that may carry a `#[autozng::walk_filter(...)]`.
pub trait WalkFilterTarget {
    /// Store `filter` for this item (or for the whole crate, on the crate root).
    fn apply_walk_filter(&self, filter: WalkFilter, span: Span, ctx: &AutozngContext);
}

#[derive(Debug, Clone)]
pub struct WalkFilterAttr {
    filter: WalkFilter,
    span: Span,
}

const WALK_FILTER_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "walk_filter"]);

const FILTER_ROOTS: &[&str] = &["all", "none", "include", "exclude"];
const RULE_NAMES: &[&str] = &[
    "Fields",
    "Variants",
    "InherentMethods",
    "TraitMethods",
    "Named",
    "Receiver",
    "Visibility",
];
const RECEIVER_NAMES: &[&str] = &["Value", "Ref", "RefMut", "Static"];
const VISIBILITY_NAMES: &[&str] = &["Public", "PublicInternal", "Private"];

impl Attr for WalkFilterAttr {
    type Target = dyn WalkFilterTarget;

    fn path(&self) -> AttrPath {
        WALK_FILTER_PATH
    }

    fn for_child(&self, _item: &Self::Target) -> Option<Self> {
        // The filter applies to the whole subtree; a child that carries its own filter
        // replaces this one in `merge`.
        Some(self.clone())
    }

    fn merge(&mut self, other: Self) {
        // `self` is the inherited filter and `other` the child's own; the child wins.
        *self = other;
    }

    fn visit(&self, item: &Self::Target, ctx: &AutozngContext) {
        item.apply_walk_filter(self.filter.clone(), self.span, ctx);
    }
}

#[derive(Debug, Default)]
pub struct WalkFilterParser {
    roots: Option<Vec<WalkFilterRoot>>,
    span: Option<Span>,
}

impl WalkFilterParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        self.span = Some(cx.span);
        let Some(list) = cx.expect_list(args, cx.span) else {
            return;
        };
        if list.is_empty() {
            cx.expected_at_least_one_argument(cx.span);
            return;
        }
        let mut roots = Vec::new();
        let mut valid = true;
        for arg in list.mixed() {
            if let Some(root) = Self::parse_root(cx, arg) {
                roots.push(root);
            } else {
                valid = false;
            }
        }
        self.roots = valid.then_some(roots);
    }

    fn parse_root(cx: &mut AcceptContext, arg: &MetaItemOrLitParser) -> Option<WalkFilterRoot> {
        let Some(meta) = arg.meta_item() else {
            cx.expected_not_literal(arg.span());
            return None;
        };
        let Some(name) = meta.path().as_ident() else {
            cx.expected_identifier(meta.span());
            return None;
        };
        match name {
            "all" => {
                cx.expect_no_args(meta.args());
                Some(WalkFilterRoot::All)
            }
            "none" => {
                cx.expect_no_args(meta.args());
                Some(WalkFilterRoot::None)
            }
            "include" => Self::parse_rules(cx, meta, false),
            "exclude" => Self::parse_rules(cx, meta, true),
            _ => {
                cx.expected_specific_argument(meta.span(), FILTER_ROOTS);
                None
            }
        }
    }

    fn parse_rules(
        cx: &mut AcceptContext,
        meta: &MetaItemParser,
        exclude: bool,
    ) -> Option<WalkFilterRoot> {
        let list = cx.expect_list(meta.args(), meta.span())?;
        let mut rules = Vec::new();
        let mut valid = true;
        for arg in list.mixed() {
            if let Some(rule) = Self::parse_rule(cx, arg) {
                rules.push(rule);
            } else {
                valid = false;
            }
        }
        valid.then_some(if exclude {
            WalkFilterRoot::Exclude(rules)
        } else {
            WalkFilterRoot::Include(rules)
        })
    }

    fn parse_rule(cx: &mut AcceptContext, arg: &MetaItemOrLitParser) -> Option<WalkRule> {
        if let Some(meta) = arg.meta_item_no_args() {
            let Some(name) = meta.path().as_ident() else {
                cx.expected_identifier(meta.span());
                return None;
            };
            return match name {
                "Fields" => Some(WalkRule::Fields),
                "Variants" => Some(WalkRule::Variants),
                "InherentMethods" => Some(WalkRule::InherentMethods),
                "TraitMethods" => Some(WalkRule::TraitMethods(None)),
                _ => {
                    cx.expected_specific_argument(meta.span(), RULE_NAMES);
                    None
                }
            };
        }

        let Some(meta) = arg.meta_item() else {
            cx.expected_not_literal(arg.span());
            return None;
        };
        let Some(name) = meta.path().as_ident() else {
            cx.expected_identifier(meta.span());
            return None;
        };
        match name {
            "TraitMethods" => {
                let path = Self::single_string(cx, meta)?;
                Some(WalkRule::TraitMethods(Some(path)))
            }
            "Named" => {
                let name = Self::single_string(cx, meta)?;
                Some(WalkRule::Named(name))
            }
            "Receiver" => {
                let (receiver, span) = Self::nested_word(cx, meta)?;
                match receiver {
                    "Value" => Some(WalkRule::Receiver(ReceiverKind::Value)),
                    "Ref" => Some(WalkRule::Receiver(ReceiverKind::Ref)),
                    "RefMut" => Some(WalkRule::Receiver(ReceiverKind::RefMut)),
                    "Static" => Some(WalkRule::Receiver(ReceiverKind::Static)),
                    _ => {
                        cx.expected_specific_argument(span, RECEIVER_NAMES);
                        None
                    }
                }
            }
            "Visibility" => {
                let (visibility, span) = Self::nested_word(cx, meta)?;
                match visibility {
                    "Public" => Some(WalkRule::Visibility(VisibilityKind::Public)),
                    "PublicInternal" => Some(WalkRule::Visibility(VisibilityKind::PublicInternal)),
                    "Private" => Some(WalkRule::Visibility(VisibilityKind::Private)),
                    _ => {
                        cx.expected_specific_argument(span, VISIBILITY_NAMES);
                        None
                    }
                }
            }
            _ => {
                cx.expected_specific_argument(meta.span(), RULE_NAMES);
                None
            }
        }
    }

    fn single_string(cx: &mut AcceptContext, meta: &MetaItemParser) -> Option<String> {
        let list = cx.expect_list(meta.args(), meta.span())?;
        let inner = cx.expect_single(list)?;
        cx.expect_string_literal(inner).map(|value| value.value)
    }

    fn nested_word<'a>(
        cx: &mut AcceptContext,
        meta: &'a MetaItemParser,
    ) -> Option<(&'a str, Span)> {
        let list = cx.expect_list(meta.args(), meta.span())?;
        let inner = cx.expect_single(list)?;
        let Some(inner_meta) = inner.meta_item_no_args() else {
            cx.expected_not_literal(inner.span());
            return None;
        };
        let Some(word) = inner_meta.path().as_ident() else {
            cx.expected_identifier(inner_meta.span());
            return None;
        };
        Some((word, inner_meta.span()))
    }
}

impl AttributeParser for WalkFilterParser {
    type Attr = WalkFilterAttr;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        WALK_FILTER_PATH,
        template!(List: FILTER_ROOTS, "https://link/to/docs"),
        Self::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        Some(WalkFilterAttr {
            filter: WalkFilter::new(self.roots?),
            span: self.span?,
        })
    }
}
