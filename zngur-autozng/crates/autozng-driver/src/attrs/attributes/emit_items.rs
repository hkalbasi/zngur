use rustc_attr_parsing::template;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttributeParser},
        },
    },
    ctx::AutozngContext,
};

pub trait EmitItemsTarget {
    /// Process the `const` tagged `#[autozng::emit_items]`.
    ///
    /// `emit_traits` is the parsed value of the attribute's `emit_traits = true|false` argument
    /// (default `false`): when set, the enumerated items are treated as concrete *trait* export
    /// targets (`dyn Trait<ConcreteArgs>`) rather than as types.
    fn emit(&self, _emit_traits: bool, _ctx: &AutozngContext) {}
}

/// `#[autozng::emit_items]` / `#[autozng::emit_items(emit_traits = true)]` on a `const`.
///
/// The `const` item (through [`EmitItemsTarget`]) is the "attributed item": its own span is what
/// the enumerated items record, so the marker carries no span of its own.
#[derive(Debug, Clone)]
pub struct EmitItems {
    /// `emit_traits = true` was given: enumerated `dyn Trait<ConcreteArgs>` items name concrete
    /// trait instantiations to export (a generic trait is only representable in `.zng` once all
    /// its type parameters are known). Defaults to `false`.
    pub emit_traits: bool,
}

const EMIT_ITEMS_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "emit_items"]);

const EMIT_TRAITS_KEY: &str = "emit_traits";

impl Attr for EmitItems {
    type Target = dyn EmitItemsTarget;
    fn path(&self) -> AttrPath {
        EMIT_ITEMS_PATH
    }

    fn visit(&self, item: &dyn EmitItemsTarget, ctx: &AutozngContext) {
        item.emit(self.emit_traits, ctx);
    }
}

#[derive(Debug, Default)]
pub struct EmitItemsParser {
    valid: bool,
    emit_traits: bool,
}

impl EmitItemsParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        match args {
            ArgParser::NoArgs => self.valid = true,
            ArgParser::List(list) => {
                let mut valid = true;
                let mut seen_emit_traits = false;
                for arg in list.mixed() {
                    let Some((key, value)) =
                        cx.expect_name_value(arg, arg.span(), Some(EMIT_TRAITS_KEY))
                    else {
                        valid = false;
                        continue;
                    };
                    if key.value != EMIT_TRAITS_KEY {
                        cx.expected_specific_argument(key.span, &[EMIT_TRAITS_KEY]);
                        valid = false;
                        continue;
                    }
                    if seen_emit_traits {
                        cx.duplicate_key(key.span, EMIT_TRAITS_KEY);
                        valid = false;
                        continue;
                    }
                    seen_emit_traits = true;

                    let Some(value) = cx.expect_bool_literal(value) else {
                        valid = false;
                        continue;
                    };
                    self.emit_traits = value.value;
                }
                self.valid = valid;
            }
            // `#[autozng::emit_items = true]` is not a valid form.
            ArgParser::NameValue(_) => {
                cx.expected_list_or_no_args(cx.span);
            }
        }
    }
}

impl AttributeParser for EmitItemsParser {
    type Attr = EmitItems;
    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        EMIT_ITEMS_PATH,
        template!(
            Word,
            List: &["emit_traits = true", "emit_traits = false"],
            "https://link/to/docs"
        ),
        EmitItemsParser::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        self.valid.then_some(EmitItems {
            emit_traits: self.emit_traits,
        })
    }
}
