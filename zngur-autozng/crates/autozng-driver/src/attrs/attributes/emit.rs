use std::{collections::HashMap, rc::Rc};

use itertools::Itertools;
use rustc_attr_parsing::template;
use rustc_span::Span;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath, DynAttr,
        parse::{
            context::AcceptContext,
            parser::{AcceptAttrMap, ArgParser, AttributeParser, ExpectIdent},
        },
    },
    config::DefaultEmitPolicy,
    ctx::AutozngContext,
    rustc_ext::Visibility,
};

const EMIT_NAME: &str = "emit";
const EMIT_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, EMIT_NAME]);

const SKIP_NAME: &str = "skip";
const SKIP_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, SKIP_NAME]);

const EMIT_PATH_ACTIONS: &[(AttrPath, EmitAction)] =
    &[(EMIT_PATH, EmitAction::Emit), (SKIP_PATH, EmitAction::Skip)];

/// Which item an `emit`/`skip` entry applies to.
///
/// `This` means the annotated item itself (bare `#[autozng::emit]` or the `self` keyword);
/// `Name` names a child field / associated item, e.g. `emit(third, forth)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EmitItem {
    This,
    Name(String),
}

/// The forced emission state for an item.
///
/// `Default` defers to `Config::default_emit_policy` + visibility; `Emit`/`Skip` override.
/// Combining (via `|`) lets more specific attributes win over less specific ones.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EmitAction {
    #[default]
    Default,
    Emit,
    Skip,
}

impl EmitAction {
    /// should emit for this action given the visibility of the item
    pub fn emit_for_visibility(&self, vis: Visibility, policy: DefaultEmitPolicy) -> bool {
        match (self, vis, policy) {
            // emit action
            (EmitAction::Emit, Visibility::Public | Visibility::PublicInternal, _) => true,
            // default action
            (EmitAction::Default, Visibility::Public, DefaultEmitPolicy::PubVis) => true,
            (
                EmitAction::Default,
                Visibility::Public | Visibility::PublicInternal,
                DefaultEmitPolicy::PubCrateVis,
            ) => true,
            (EmitAction::Default, Visibility::PublicInternal, DefaultEmitPolicy::PubVis) => false,
            (EmitAction::Default, _, DefaultEmitPolicy::None) => false,
            // skip action or private
            (EmitAction::Skip, _, _) | (_, Visibility::PrivateAPI, _) => false,
        }
    }

    /// [`EmitAction::emit_for_visibility`], warning at `span` when the item was explicitly tagged
    /// `emit` yet is not visible from the crate root (so the generated code cannot see it).
    ///
    /// An item that is only *implicitly* selected (the `Default` action) is skipped silently —
    /// only an explicit `emit` on an unreachable item is surprising enough to warn about.
    pub fn emit_for_visibility_or_warn(
        &self,
        vis: Visibility,
        policy: DefaultEmitPolicy,
        span: Span,
    ) -> bool {
        if matches!(self, EmitAction::Emit) && vis == Visibility::PrivateAPI {
            crate::error::warn_private_item_skipped(span);
            return false;
        }
        self.emit_for_visibility(vis, policy)
    }

    /// should emit for this action when visibility is not applicable for the item (statements etc.)
    pub fn emit_visibility_na(&self) -> bool {
        match self {
            EmitAction::Default | EmitAction::Emit => true,
            EmitAction::Skip => false,
        }
    }
}

impl core::ops::BitOr for EmitAction {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Default, rhs) => rhs,
            (_, rhs @ Self::Skip | rhs @ Self::Emit) => rhs,
            (lhs, Self::Default) => lhs,
        }
    }
}
impl core::ops::BitOrAssign for EmitAction {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs
    }
}

pub trait EmitTarget {
    /// Process the item with the given emit action.
    ///
    /// Process the item with the given emit action.
    ///
    /// `span` is the span of the `#[autozng::emit]` / `#[autozng::skip]` attribute that selected
    /// the item (or the item's own span for an implicit emit), used to report an unreachable item
    /// that was explicitly tagged `emit`. Recording uses [`EmitTarget::emit_span`] (the item's own
    /// span) so render diagnostics can point back at the item that was selected.
    fn emit(&self, action: EmitAction, span: Span, ctx: &AutozngContext) -> bool;

    fn name(&self) -> String;

    /// The item's source span, used as the span of its implicit `Emit` attribute and as the
    /// `span` recorded by [`AutozngContext::record_ty`] & friends.
    fn emit_span(&self) -> Span;

    /// if the item has an implicit emit action.
    /// normally `None` or `Some(EmitAction::Default)` to follow visibility defaults
    fn implicit_action(&self) -> Option<EmitAction> {
        None
    }

    /// get the implicit Emit attr
    fn implicit_emit_attr(&self) -> Option<Rc<dyn DynAttr>> {
        self.implicit_action().map(|action| {
            Rc::new(Emit::new_implicit_emit(action, self.emit_span())) as Rc<dyn DynAttr>
        })
    }
}

#[derive(Debug, Clone)]
pub struct Emit {
    items: HashMap<EmitItem, EmitAction>,
    /// Span of the attribute that created this `Emit`: the `#[autozng::emit(...)]` /
    /// `#[autozng::skip(...)]`, or the target's own span for an implicit emit.
    span: Span,
}

impl Emit {
    fn new_implicit_emit(action: EmitAction, span: Span) -> Self {
        Emit {
            items: HashMap::from([(EmitItem::This, action)]),
            span,
        }
    }
}

impl Attr for Emit {
    type Target = dyn EmitTarget;

    fn path(&self) -> AttrPath {
        // `emit` and `skip` share a single canonical path so that `process_attrs` merges
        // them (together with the implicit `Default` action) into one attribute. The actual
        // action per item is carried in `items`, and `EmitAction`'s `|` decides which wins.
        EMIT_PATH
    }

    fn for_child(&self, item: &dyn EmitTarget) -> Option<Self> {
        if self
            .items
            .iter()
            .any(|(item, _)| matches!(item, EmitItem::Name(_)))
        {
            Some(Emit {
                items: self
                    .items
                    .iter()
                    .filter_map(|(match_item, action)| {
                        if matches!(match_item, EmitItem::Name(name) if name == &item.name() ) {
                            Some((EmitItem::This, *action))
                        } else {
                            None
                        }
                    })
                    .unique()
                    .collect(),
                span: self.span,
            })
        } else {
            None
        }
    }

    fn merge(&mut self, other: Self) {
        // An explicit `#[autozng::emit(...)]` / `#[autozng::skip(...)]` is merged into the
        // implicit emit; its own span (the attribute) is more precise than the implicit emit's
        // (the item's own span), so it is the one reported.
        self.span = other.span;
        // downcast then unwrap or clone the inner EmitAttr to merge the path list
        for (item, action) in other.items {
            let entry = self.items.entry(item).or_insert(action);
            *entry |= action;
        }
    }

    fn visit(&self, item: &dyn EmitTarget, ctx: &AutozngContext) {
        if let Some(action) = self.items.get(&EmitItem::This) {
            item.emit(*action, self.span, ctx);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EmitParser {
    items: Option<Vec<(EmitItem, EmitAction)>>,
    span: Option<Span>,
}

impl AttributeParser for EmitParser {
    type Attr = Emit;
    const ATTRIBUTES: AcceptAttrMap<Self> = &[
        (
            EMIT_PATH,
            template!(Word, List: &["self", "field", "assoc_item"], "https://link/to/docs"),
            Self::parse,
        ),
        (
            SKIP_PATH,
            template!(Word, List: &["self", "field", "assoc_item"], "https://link/to/docs"),
            Self::parse,
        ),
    ];

    fn finalize(self, _target: &dyn EmitTarget) -> Option<Self::Attr> {
        if let EmitParser {
            items: Some(items),
            span: Some(span),
        } = self
        {
            return Some(Emit {
                items: items.into_iter().collect(),
                span,
            });
        }
        None
    }
}

impl EmitParser {
    pub fn parse(&mut self, ac: &mut AcceptContext, args: &ArgParser) {
        self.span = Some(ac.span);
        let Some(action) = EMIT_PATH_ACTIONS
            .iter()
            .find_map(|(path, action)| {
                if path == ac.attr_path {
                    Some(action)
                } else {
                    None
                }
            })
            .copied()
        else {
            ac.dcx
                .struct_span_err(
                    ac.span,
                    format!("BUG! unknown emit action for path `{}` ", ac.attr_path),
                )
                .emit();
            return;
        };
        self.items = match args {
            ArgParser::NoArgs => Some(vec![(EmitItem::This, action)]),
            ArgParser::List(args) => {
                let arg_results = args
                    .mixed()
                    .map(|arg| {
                        if let Some(ident) = arg.expect_ident(ac) {
                            Ok(if ident == "self" {
                                (EmitItem::This, action)
                            } else {
                                (EmitItem::Name(ident.to_owned()), action)
                            })
                        } else {
                            Err(())
                        }
                    })
                    .collect::<Vec<_>>();
                arg_results.into_iter().collect::<Result<Vec<_>, _>>().ok()
            }
            ArgParser::NameValue(_) => {
                ac.expected_list_or_no_args(ac.span);
                None
            }
        };
    }
}

const EMIT_ON_IMPL_PATH: AttrPath =
    AttrPath::from_static(&[AUTOZNG_NAMESPACE, "emit_on_implementers"]);

/// Target for `#[autozng::emit_on_implementers]`: traits and trait impl blocks.
///
/// Semantics: record the trait's functions so they are emitted on every type that implements the
/// trait (and satisfies the trait's / impl's clauses), without the type having to enumerate them
/// in an `emit_items` const.
pub trait EmitOnImplTarget {
    /// Record this trait / impl into `AutozngContextData::emit_for`.
    ///
    /// `span` is the span of the `#[autozng::emit_on_implementers]` attribute, used to report a
    /// private trait that has to be skipped.
    fn emit_on_impl(&self, span: Span, ctx: &AutozngContext);
}

#[derive(Debug, Copy, Clone)]
pub struct EmitOnImpl {
    /// Span of the `#[autozng::emit_on_implementers]` attribute.
    span: Span,
}

impl Attr for EmitOnImpl {
    type Target = dyn EmitOnImplTarget;

    fn path(&self) -> AttrPath {
        EMIT_ON_IMPL_PATH
    }

    fn visit(&self, item: &Self::Target, ctx: &AutozngContext) {
        item.emit_on_impl(self.span, ctx);
    }
}

#[derive(Debug, Default)]
pub struct EmitOnImplParser {
    valid: bool,
    span: Option<Span>,
}

impl EmitOnImplParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        self.valid = cx.expect_no_args(args).is_some();
        self.span = Some(cx.span);
    }
}

impl AttributeParser for EmitOnImplParser {
    type Attr = EmitOnImpl;
    const ATTRIBUTES: AcceptAttrMap<Self> = &[(
        EMIT_ON_IMPL_PATH,
        template!(Word, "https://link/to/docs"),
        EmitOnImplParser::parse,
    )];

    fn finalize(self, _target: &dyn EmitOnImplTarget) -> Option<Self::Attr> {
        if !self.valid {
            return None;
        }
        Some(EmitOnImpl { span: self.span? })
    }
}
