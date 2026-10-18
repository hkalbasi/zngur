use std::path::PathBuf;

/// How an item with no explicit `emit`/`skip` attribute is treated, based on its visibility.
///
/// Ordering is least-to-most permissive: `None` emits nothing by default, `PubCrateVis`
/// also emits `pub(crate)`, `PubVis` emits only fully `pub` items.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DefaultEmitPolicy {
    #[default]
    PubVis,
    #[allow(dead_code)] // selectable policy; `run` only exposes `PubVis`/`None` so far
    PubCrateVis,
    None,
}

/// Where the generated `.zng` file should be written.
///
/// Populated by [`crate::run`] from the command line (it intercepts `--output-zng` and
/// records `--out-dir` before handing the remaining arguments to `rustc`). `cargo-autozng`
/// is expected to forward the same flags through `RUSTC_WORKSPACE_WRAPPER`.
#[derive(Debug, Default, Clone)]
pub struct OutputConfig {
    /// Explicit output file from `--output-zng <path>` (or `--output-zng=<path>`).
    ///
    /// When set this wins over [`OutputConfig::out_dir`] and the crate name.
    pub output_zng: Option<PathBuf>,
    /// The directory passed as rustc's `--out-dir`, if any.
    ///
    /// When no `--output-zng` is given the `.zng` is written here (or into the current
    /// directory, rustc's own default, when this is `None`) using the crate name as the
    /// file stem. rustc's `-o` argument is intentionally *not* consulted.
    pub out_dir: Option<PathBuf>,
}

/// Driver configuration.
///
/// `crate::run` builds this from the command line (output location); `#[autozng::policy]` adjusts
/// it during analysis (see [`crate::attrs::attributes::policy`]).
#[derive(Debug, Default, Clone)]
pub struct Config {
    pub default_emit_policy: DefaultEmitPolicy,
    pub use_doc_comment_tool_attrs: bool,
    pub output: OutputConfig,
    /// `#![autozng::policy(implicitly_emit_external_interfaces)]`.
    ///
    /// When set, the recursive interface pass also walks the children of a **non-local** ADT (see
    /// `analyze::visit_emitted_ty`), so an external type reached through an interface exposes its
    /// own fields/methods rather than just its `type` block.
    pub implicitly_emit_external_interfaces: bool,
}
