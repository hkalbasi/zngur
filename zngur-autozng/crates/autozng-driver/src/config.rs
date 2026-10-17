/// How an item with no explicit `emit`/`skip` attribute is treated, based on its visibility.
///
/// Ordering is least-to-most permissive: `None` emits nothing by default, `PubCrateVis`
/// also emits `pub(crate)`, `PubVis` emits only fully `pub` items.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DefaultEmitPolicy {
    #[default]
    PubVis,
    PubCrateVis,
    None,
}

/// Driver configuration.
///
/// Currently always `Config::default` (`RustcPublic::new` has no way to set it); the
/// `#[autozng::policy]` attribute is a placeholder and does not modify this yet.
#[derive(Debug, Default)]
pub struct Config {
    pub default_emit_policy: DefaultEmitPolicy,
    pub use_doc_comment_tool_attrs: bool,
}
