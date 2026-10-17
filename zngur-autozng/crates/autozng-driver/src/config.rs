#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DefaultEmitPolicy {
    #[default]
    PubVis,
    PubCrateVis,
    None,
}

#[derive(Debug, Default)]
pub struct Config {
    pub default_emit_policy: DefaultEmitPolicy,
    pub use_doc_comment_tool_attrs: bool,
}
