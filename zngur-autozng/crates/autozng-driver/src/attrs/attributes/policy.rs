use rustc_attr_parsing::template;

use crate::attrs::AttrPath;

use crate::attrs::parse::context::AcceptContext;
use crate::attrs::parse::parser::{ArgParser, AttrtibuteParser};
use crate::{
    attrs::{AUTOZNG_NAMESPACE, Attr},
    ctx::AutozngContext,
};

pub trait PolicyTarget {}

#[derive(Debug, Copy, Clone)]
pub struct Policy;

const POLICY_NAME: &str = "policy";
const POLICY_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, POLICY_NAME]);

impl Attr for Policy {
    type Target = dyn PolicyTarget;

    fn path(&self) -> AttrPath {
        POLICY_PATH
    }

    fn visit(&self, item: &Self::Target, ctx: &AutozngContext) {
        // TODO:
    }
}

#[derive(Debug, Default)]
pub struct PolicyParser {
    valid: bool,
}

impl AttrtibuteParser for PolicyParser {
    type Attr = Policy;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[(
        POLICY_PATH,
        template!(List: &["policy_name1, policy_name2"], "https:://link/to/docs"),
        Self::parse,
    )];

    fn finalize(self, target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        None
    }
}

impl PolicyParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {}
}
