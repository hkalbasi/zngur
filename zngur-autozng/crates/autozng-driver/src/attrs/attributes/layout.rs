//! `#[autozng::layout(...)]` — override the `.zng` layout policy of a type.
//!
//! By default a type is emitted with `#layout(size = <real size>, align = <real align>)`
//! ([`LayoutPolicy::StackAllocated`]) derived from its real layout. This attribute lets a
//! downstream user choose another policy; the choice is recorded on the type's
//! [`MetadataRecord`](crate::metadata::MetadataRecord) and applied during spec generation
//! (`render::build_type`), replacing the automatic layout.
//!
//! Exactly one policy may be given per type:
//! - `stack(size = N, align = N)` — the values **must** match the type's real size and alignment;
//! - `conservative(size = N, align = N)` — the values must be *at least* the real size /
//!   alignment (an alignment that is not a power of two is warned about);
//! - `heap_allocated`;
//! - `only_by_ref`.
//!
//! A mismatch is an error pointing at the attribute. A generic ADT has no single real layout, so
//! its override is recorded without validation (and applied to every concrete instantiation).

use rustc_ast::{LitKind, MetaItemLit};
use rustc_attr_parsing::template;
use rustc_span::Span;
use zngur_def::LayoutPolicy;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttributeParser, MetaItemParser, NameValueParser},
        },
    },
    ctx::AutozngContext,
    error::{emit_error_for_span, emit_warning_for_span},
    metadata::{IntoMetadataKey, LayoutOverride},
};

/// The policy names accepted as the single argument of `#[autozng::layout(...)]`.
const LAYOUT_KINDS: &[&str] = &["stack", "conservative", "heap_allocated", "only_by_ref"];

const SIZE_KEY: &str = "size";
const ALIGN_KEY: &str = "align";

/// A type a `#[autozng::layout(...)]` attribute can be attached to.
///
/// Implemented for ADTs. [`LayoutTarget::real_layout`] is used to validate an explicit
/// `stack` / `conservative` size and alignment; a type whose layout is not computable (a generic
/// ADT) is recorded without validation.
pub trait LayoutTarget: IntoMetadataKey {
    /// The type's real layout, or `None` when it is not computable (a generic ADT) or the type is
    /// unsized (so there is no size to compare against).
    fn real_layout(&self) -> Option<rustc_public::abi::Layout>;

    /// The type's name, for diagnostics.
    fn layout_type_name(&self) -> String;
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    policy: LayoutPolicy,
    /// Span of the `#[autozng::layout(...)]` attribute, used for mismatch diagnostics.
    span: Span,
}

const LAYOUT_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "layout"]);

impl Layout {
    /// Check an explicit `stack` / `conservative` size and alignment against the type's real
    /// layout. `heap_allocated` / `only_by_ref` carry no values to check.
    fn validate(&self, item: &dyn LayoutTarget) {
        let Some(real) = item.real_layout() else {
            return;
        };
        let shape = real.shape();
        let real_size = shape.size.bytes();
        let real_align = shape.abi_align as usize;
        let name = item.layout_type_name();
        match self.policy {
            LayoutPolicy::StackAllocated { size, align } => {
                if size != real_size {
                    emit_error_for_span(
                        self.span,
                        format!(
                            "autozng: `stack` layout size for `{name}` must match its real size \
                             {real_size} (found {size})"
                        ),
                    );
                }
                if align != real_align {
                    emit_error_for_span(
                        self.span,
                        format!(
                            "autozng: `stack` layout align for `{name}` must match its real align \
                             {real_align} (found {align})"
                        ),
                    );
                }
            }
            LayoutPolicy::Conservative { size, align } => {
                if size < real_size {
                    emit_error_for_span(
                        self.span,
                        format!(
                            "autozng: `conservative` layout size for `{name}` must be at least its \
                             real size {real_size} (found {size})"
                        ),
                    );
                }
                if align < real_align {
                    emit_error_for_span(
                        self.span,
                        format!(
                            "autozng: `conservative` layout align for `{name}` must be at least its \
                             real align {real_align} (found {align})"
                        ),
                    );
                }
                if !align.is_power_of_two() {
                    emit_warning_for_span(
                        self.span,
                        format!(
                            "autozng: `conservative` layout align {align} for `{name}` is not a \
                             power of two"
                        ),
                    );
                }
            }
            LayoutPolicy::HeapAllocated | LayoutPolicy::OnlyByRef => {}
        }
    }
}

impl Attr for Layout {
    type Target = dyn LayoutTarget;

    fn path(&self) -> AttrPath {
        LAYOUT_PATH
    }

    fn merge(&mut self, other: Self) {
        // The framework merges duplicate paths on one item; only one layout policy is allowed.
        emit_error_for_span(
            other.span,
            "autozng: only one `autozng::layout` policy is allowed per type",
        );
    }

    fn visit(&self, item: &dyn LayoutTarget, ctx: &AutozngContext) {
        self.validate(item);
        ctx.with_metadata_mut(item, |mut metadata| {
            metadata.layout = Some(LayoutOverride {
                policy: self.policy,
                span: self.span,
            });
        });
    }
}

#[derive(Debug, Default)]
pub struct LayoutParser {
    policy: Option<LayoutPolicy>,
    span: Option<Span>,
}

impl LayoutParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        self.span = Some(cx.span);
        let Some(list) = cx.expect_list(args, cx.span) else {
            return;
        };
        // Exactly one policy argument is allowed.
        let Some(arg) = cx.expect_single(list) else {
            return;
        };
        let Some(meta) = arg.meta_item() else {
            cx.expected_specific_argument(arg.span(), LAYOUT_KINDS);
            return;
        };
        let Some(kind) = meta.path().as_ident() else {
            cx.expected_specific_argument(meta.span(), LAYOUT_KINDS);
            return;
        };
        self.policy = match kind {
            "stack" => Self::parse_sized(cx, meta, false),
            "conservative" => Self::parse_sized(cx, meta, true),
            "heap_allocated" => {
                cx.expect_no_args(meta.args());
                Some(LayoutPolicy::HeapAllocated)
            }
            "only_by_ref" => {
                cx.expect_no_args(meta.args());
                Some(LayoutPolicy::OnlyByRef)
            }
            _ => {
                cx.expected_specific_argument(meta.span(), LAYOUT_KINDS);
                None
            }
        };
    }

    /// Parse `stack(size = N, align = N)` / `conservative(size = N, align = N)`.
    fn parse_sized(
        cx: &mut AcceptContext,
        meta: &MetaItemParser,
        conservative: bool,
    ) -> Option<LayoutPolicy> {
        let list = cx.expect_list(meta.args(), meta.span())?;
        let mut size = None;
        let mut align = None;
        let mut valid = true;
        for arg in list.mixed() {
            let Some((key, value)) = cx.expect_name_value(arg, arg.span(), None) else {
                valid = false;
                continue;
            };
            match key.value {
                SIZE_KEY => {
                    if size.is_some() {
                        cx.duplicate_key(key.span, SIZE_KEY);
                        valid = false;
                    } else if let Some(parsed) = expect_usize(cx, value) {
                        size = Some(parsed);
                    } else {
                        valid = false;
                    }
                }
                ALIGN_KEY => {
                    if align.is_some() {
                        cx.duplicate_key(key.span, ALIGN_KEY);
                        valid = false;
                    } else if let Some(parsed) = expect_usize(cx, value) {
                        align = Some(parsed);
                    } else {
                        valid = false;
                    }
                }
                _ => {
                    cx.expected_specific_argument(key.span, &[SIZE_KEY, ALIGN_KEY]);
                    valid = false;
                }
            }
        }
        if !valid {
            return None;
        }
        let (Some(size), Some(align)) = (size, align) else {
            if size.is_none() {
                cx.missing_name_value(cx.span, SIZE_KEY);
            }
            if align.is_none() {
                cx.missing_name_value(cx.span, ALIGN_KEY);
            }
            return None;
        };
        Some(if conservative {
            LayoutPolicy::Conservative { size, align }
        } else {
            LayoutPolicy::StackAllocated { size, align }
        })
    }
}

/// Read a `usize` from a name-value pair's literal, erroring if it is not an integer.
fn expect_usize(cx: &mut AcceptContext, value: &NameValueParser) -> Option<usize> {
    let lit: &MetaItemLit = value.value_as_lit();
    match lit.kind {
        LitKind::Int(value, _) => usize::try_from(value.get()).ok().or_else(|| {
            cx.expected_integer_literal(lit.span);
            None
        }),
        _ => {
            cx.expected_integer_literal(lit.span);
            None
        }
    }
}

impl AttributeParser for LayoutParser {
    type Attr = Layout;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        LAYOUT_PATH,
        template!(
            List: &[
                "stack(size = <usize>, align = <usize>)",
                "conservative(size = <usize>, align = <usize>)",
                "heap_allocated",
                "only_by_ref"
            ],
            "https://link/to/docs"
        ),
        LayoutParser::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        Some(Layout {
            policy: self.policy?,
            span: self.span?,
        })
    }
}
