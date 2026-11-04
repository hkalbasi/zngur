//! `#[autozng::policy]` — adjusts the driver [`Config`](crate::config::Config) from the crate
//! root.
//!
//! Policies are applied when the crate root's attributes are visited
//! (`CrateAnalysisVisitor::visit_crate`), before the interface pass drains its queue. The
//! attribute is only accepted on the crate root (`PolicyTarget` is implemented for
//! [`rustc_public::Crate`]).
//!
//! `implicitly_emit_external_interfaces` makes the recursive interface pass walk the children of
//! a **non-local** ADT. The `max_*_depth` entries set the walk budgets; every walk copies the
//! applicable budget into its request metadata, so changing the config later cannot affect a
//! walk already in progress.

use rustc_attr_parsing::template;

use crate::attrs::AttrPath;

use crate::attrs::parse::context::AcceptContext;
use crate::attrs::parse::parser::{ArgParser, AttributeParser};
use crate::{
    attrs::{AUTOZNG_NAMESPACE, Attr},
    ctx::AutozngContext,
};

pub trait PolicyTarget {}

/// A concrete `#[autozng::policy(...)]` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    /// `implicitly_emit_external_interfaces`: walk the children of non-local ADTs reached through
    /// an interface.
    ImplicitlyEmitExternalInterfaces,
    /// `max_walk_depth = N`: ordinary interface-walk depth budget.
    MaxWalkDepth(usize),
    /// `max_external_depth = N`: non-local hop budget.
    MaxExternalDepth(usize),
    /// `max_trait_impl_depth = N`: `emit_traits` walk budget.
    MaxTraitImplDepth(usize),
    /// `max_trait_interface_depth = N`: `emit_on_implementers` walk budget.
    MaxTraitInterfaceDepth(usize),
    /// `max_self_ref_depth = N`: budget for a child whose base type is already an ancestor.
    MaxSelfRefDepth(usize),
}

const POLICY_FLAG: &str = "implicitly_emit_external_interfaces";

/// Constructor for a name-value policy's setting.
type PolicyCtor = fn(usize) -> PolicyKind;

/// The known name-value policies and the constructor for their setting.
const POLICY_VALUES: &[(&str, PolicyCtor)] = &[
    ("max_walk_depth", PolicyKind::MaxWalkDepth),
    ("max_external_depth", PolicyKind::MaxExternalDepth),
    ("max_trait_impl_depth", PolicyKind::MaxTraitImplDepth),
    (
        "max_trait_interface_depth",
        PolicyKind::MaxTraitInterfaceDepth,
    ),
    ("max_self_ref_depth", PolicyKind::MaxSelfRefDepth),
];

/// The known policies, for parsing and diagnostics.
const POLICY_NAMES: &[&str] = &[
    POLICY_FLAG,
    "max_walk_depth",
    "max_external_depth",
    "max_trait_impl_depth",
    "max_trait_interface_depth",
    "max_self_ref_depth",
];

impl PolicyKind {
    /// Apply this policy to the driver configuration.
    fn apply(self, ctx: &AutozngContext) {
        let mut config = ctx.config_mut();
        match self {
            PolicyKind::ImplicitlyEmitExternalInterfaces => {
                config.implicitly_emit_external_interfaces = true;
            }
            PolicyKind::MaxWalkDepth(depth) => config.max_walk_depth = depth,
            PolicyKind::MaxExternalDepth(depth) => config.max_external_depth = depth,
            PolicyKind::MaxTraitImplDepth(depth) => config.max_trait_impl_depth = depth,
            PolicyKind::MaxTraitInterfaceDepth(depth) => config.max_trait_interface_depth = depth,
            PolicyKind::MaxSelfRefDepth(depth) => config.max_self_ref_depth = depth,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Policy {
    policies: Vec<PolicyKind>,
}

const POLICY_NAME: &str = "policy";
const POLICY_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, POLICY_NAME]);

impl Attr for Policy {
    type Target = dyn PolicyTarget;

    fn path(&self) -> AttrPath {
        POLICY_PATH
    }

    fn visit(&self, _item: &Self::Target, ctx: &AutozngContext) {
        for policy in &self.policies {
            policy.apply(ctx);
        }
    }
}

#[derive(Debug, Default)]
pub struct PolicyParser {
    policies: Option<Vec<PolicyKind>>,
}

impl PolicyParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        let Some(list) = args.as_list() else {
            cx.expected_list(cx.span, args);
            return;
        };
        let mut policies = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut valid = true;

        for arg in list.mixed() {
            // A bare flag, e.g. `implicitly_emit_external_interfaces`.
            if let Some(meta) = arg.meta_item_no_args() {
                let Some(name) = meta.path().as_ident() else {
                    cx.expected_identifier(meta.span());
                    valid = false;
                    continue;
                };
                if name == POLICY_FLAG {
                    if seen.contains(&name) {
                        cx.duplicate_key(meta.span(), name);
                        valid = false;
                        continue;
                    }
                    seen.push(name);
                    policies.push(PolicyKind::ImplicitlyEmitExternalInterfaces);
                } else {
                    cx.expected_specific_argument(meta.span(), POLICY_NAMES);
                    valid = false;
                }
                continue;
            }

            // A `name = value` entry.
            let Some((key, value)) = cx.expect_name_value(arg, arg.span(), None) else {
                valid = false;
                continue;
            };
            if key.value == POLICY_FLAG {
                cx.expected_no_args(key.span);
                valid = false;
                continue;
            }
            let Some(&(name, constructor)) =
                POLICY_VALUES.iter().find(|(name, _)| *name == key.value)
            else {
                cx.expected_specific_argument(key.span, POLICY_NAMES);
                valid = false;
                continue;
            };
            if seen.contains(&name) {
                cx.duplicate_key(key.span, name);
                valid = false;
                continue;
            }
            let Some(depth) = cx.expect_usize_literal(value) else {
                valid = false;
                continue;
            };
            seen.push(name);
            policies.push(constructor(depth));
        }

        self.policies = valid.then_some(policies);
    }
}

impl AttributeParser for PolicyParser {
    type Attr = Policy;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        POLICY_PATH,
        template!(List: POLICY_NAMES, "https:://link/to/docs"),
        Self::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        Some(Policy {
            policies: self.policies?,
        })
    }
}
