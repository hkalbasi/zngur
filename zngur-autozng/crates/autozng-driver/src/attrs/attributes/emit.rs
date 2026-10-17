use std::{collections::HashMap, rc::Rc};

use itertools::Itertools;
use rustc_attr_parsing::template;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath, DynAttr,
        parse::{
            context::AcceptContext,
            parser::{AcceptAttrMap, ArgParser, AttrtibuteParser, ExpectIdent},
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
    /// process the item with the given emit action
    fn emit(&self, action: EmitAction, ctx: &AutozngContext) -> bool;

    fn name(&self) -> String;

    /// if the item has an implicit emit action.
    /// normally `None` or `Some(EmitAction::Default)` to follow visibility defaults
    fn implicit_action(&self) -> Option<EmitAction> {
        None
    }

    /// get the implicit Emit attr
    fn implicit_emit_attr(&self) -> Option<Rc<dyn DynAttr>> {
        self.implicit_action()
            .map(|action| Rc::new(Emit::new_implicit_emit(action)) as Rc<dyn DynAttr>)
    }
}

#[derive(Debug, Clone)]
pub struct Emit {
    items: HashMap<EmitItem, EmitAction>,
}

impl Emit {
    fn new_implicit_emit(action: EmitAction) -> Self {
        Emit {
            items: HashMap::from([(EmitItem::This, action)]),
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
            })
        } else {
            None
        }
    }

    fn merge(&mut self, other: Self) {
        // downcast then unwrap or clone the inner EmitAttr to merge the path list
        for (item, action) in other.items {
            let entry = self.items.entry(item).or_insert(action);
            *entry |= action;
        }
    }

    fn visit(&self, item: &dyn EmitTarget, ctx: &AutozngContext) {
        if let Some(action) = self.items.get(&EmitItem::This) {
            item.emit(*action, ctx);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EmitParser {
    items: Option<Vec<(EmitItem, EmitAction)>>,
}

impl AttrtibuteParser for EmitParser {
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

    fn finalize(self, target: &dyn EmitTarget) -> Option<Self::Attr> {
        if let EmitParser {
            items: Some(items),
        } = self
        {
            return Some(Emit {
                items: items.into_iter().collect(),
            });
        }
        None
    }
}

impl EmitParser {
    pub fn parse(&mut self, ac: &mut AcceptContext, args: &ArgParser) {
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
/// Semantics: emit the eligible (public or `emit`-tagged) methods of this trait / impl for
/// every implementing type found. Parsing works; [`EmitOnImpl::visit`] is still a stub.
pub trait EmitOnImplTarget {}

#[derive(Debug, Copy, Clone)]
pub struct EmitOnImpl;

impl Attr for EmitOnImpl {
    type Target = dyn EmitOnImplTarget;

    fn path(&self) -> AttrPath {
        EMIT_ON_IMPL_PATH
    }

    fn visit(&self, item: &Self::Target, ctx: &AutozngContext) {
        // TODO: emit the trait's / impl's eligible methods onto each implementing type.
    }
}

#[derive(Debug, Default)]
pub struct EmitOnImplParser {
    valid: bool,
}

impl EmitOnImplParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        self.valid = cx.expect_no_args(args).is_some();
    }
}

impl AttrtibuteParser for EmitOnImplParser {
    type Attr = EmitOnImpl;
    const ATTRIBUTES: AcceptAttrMap<Self> = &[(
        EMIT_ON_IMPL_PATH,
        template!(Word, "https://link/to/docs"),
        EmitOnImplParser::parse,
    )];

    fn finalize(self, target: &dyn EmitOnImplTarget) -> Option<Self::Attr> {
        self.valid.then_some(EmitOnImpl)
    }
}
