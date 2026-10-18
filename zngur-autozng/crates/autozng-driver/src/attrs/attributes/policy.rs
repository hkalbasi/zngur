//! `#[autozng::policy]` — adjusts the driver [`Config`](crate::config::Config) from the crate
//! root.
//!
//! The only policy so far is `implicitly_emit_external_interfaces`, which makes the recursive
//! interface pass walk the children of a **non-local** ADT (see `analyze::visit_emitted_ty`), so
//! an external type reached through an interface exposes its own fields/methods instead of just
//! its `type` block. Policies are applied when the crate root's attributes are visited
//! (`CrateAnalysisVisitor::visit_crate`), before the interface pass drains its queue.

use rustc_attr_parsing::template;

use crate::attrs::AttrPath;

use crate::attrs::parse::context::AcceptContext;
use crate::attrs::parse::parser::{ArgParser, AttrtibuteParser, ExpectIdent};
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
}

/// The known policies and the names that select them (for parsing and diagnostics).
const POLICY_NAMES: &[(&str, PolicyKind)] = &[(
    "implicitly_emit_external_interfaces",
    PolicyKind::ImplicitlyEmitExternalInterfaces,
)];

impl PolicyKind {
    fn from_name(name: &str) -> Option<Self> {
        POLICY_NAMES
            .iter()
            .find_map(|(known, kind)| (*known == name).then_some(*kind))
    }

    fn names() -> Vec<&'static str> {
        POLICY_NAMES.iter().map(|(name, _)| *name).collect()
    }

    /// Apply this policy to the driver configuration.
    fn apply(self, ctx: &AutozngContext) {
        match self {
            PolicyKind::ImplicitlyEmitExternalInterfaces => {
                ctx.config_mut().implicitly_emit_external_interfaces = true;
            }
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
        for arg in list.mixed() {
            let Some(name) = arg.expect_ident(cx) else {
                continue;
            };
            match PolicyKind::from_name(name) {
                Some(policy) => policies.push(policy),
                None => {
                    cx.expected_specific_argument(arg.span(), &PolicyKind::names());
                }
            }
        }
        self.policies = Some(policies);
    }
}

impl AttrtibuteParser for PolicyParser {
    type Attr = Policy;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        POLICY_PATH,
        template!(
            List: &["implicitly_emit_external_interfaces"],
            "https:://link/to/docs"
        ),
        Self::parse,
    )];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        Some(Policy {
            policies: self.policies?,
        })
    }
}
