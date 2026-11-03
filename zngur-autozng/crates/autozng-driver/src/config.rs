use std::path::PathBuf;

use crate::walk::WalkFilter;

/// Environment variable naming the directory the generated `.zng` files are written to.
///
/// As a `RUSTC_WORKSPACE_WRAPPER` the driver is invoked by cargo with nothing but rustc
/// arguments, so the output location has to reach it out of band. The literal is the contract
/// with `cargo-autozng`, which declares the same name independently: the frontend must not
/// depend on this crate (it would drag in `rustc_private`), so the string is duplicated.
pub const ENV_OUTPUT_DIR: &str = "AUTOZNG_OUTPUT_DIR";

/// Environment variable naming an explicit `.zng` file. See [`ENV_OUTPUT_DIR`].
///
/// Only meaningful when a single crate is compiled, since every wrapper invocation writes
/// the same file otherwise.
pub const ENV_OUTPUT_ZNG: &str = "AUTOZNG_OUTPUT_ZNG";

/// Environment variable naming the sysroot to pass to `rustc` as `--sysroot`.
///
/// Belt-and-braces: the driver already loads `librustc_driver` from the toolchain it needs, so
/// the sysroot follows the loaded library; this is a second, independent route to the same
/// answer. See [`ENV_OUTPUT_DIR`] for why the name is duplicated with `cargo-autozng`.
pub const ENV_SYSROOT: &str = "AUTOZNG_SYSROOT";

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
/// records `--out-dir` before handing the remaining arguments to `rustc`) and from the
/// `AUTOZNG_OUTPUT_DIR` / `AUTOZNG_OUTPUT_ZNG` environment variables that
/// [`cargo-autozng`](../cargo_autozng/index.html) sets on the wrapper invocation.
#[derive(Debug, Default, Clone)]
pub struct OutputConfig {
    /// Explicit output file from `--output-zng <path>` (or `--output-zng=<path>`).
    ///
    /// When set this wins over every other source.
    pub output_zng: Option<PathBuf>,
    /// Directory from the `AUTOZNG_OUTPUT_DIR` environment variable (or `--output-dir`).
    ///
    /// When no explicit file is given the `.zng` is written here using the crate name as the
    /// file stem. This is the channel `cargo-autozng` uses, since a wrapper invocation has no
    /// room on the command line.
    pub output_dir: Option<PathBuf>,
    /// The directory passed as rustc's `--out-dir`, if any.
    ///
    /// When neither an explicit file nor an output directory is given the `.zng` is written
    /// here (or into the current directory, rustc's own default, when this is `None`) using the
    /// crate name as the file stem. rustc's `-o` argument is intentionally *not* consulted.
    pub out_dir: Option<PathBuf>,
}

/// Driver configuration.
///
/// `crate::run` builds this from the command line (output location); `#[autozng::policy]` adjusts
/// it during analysis (see [`crate::attrs::attributes::policy`]).
#[derive(Debug, Clone)]
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
    /// Maximum number of hops an ordinary interface walk may descend (local hops included).
    pub max_walk_depth: usize,
    /// Maximum number of non-local hops an interface walk may descend.
    ///
    /// The first `std`/`core`/`alloc` type on a branch is free (see
    /// [`crate::walk::WalkTarget`]), so an external crate's API and the standard-library types
    /// its signatures name are exposed together.
    pub max_external_depth: usize,
    /// Depth budget for an `emit_traits` walk (a trait C++ can implement).
    pub max_trait_impl_depth: usize,
    /// Depth budget for an `emit_on_implementers` walk (the trait interface attached to an
    /// emitted type).
    pub max_trait_interface_depth: usize,
    /// Depth budget for a child whose unwrapped type is already in the walk's ancestor chain.
    pub max_self_ref_depth: usize,
    /// `#![autozng::walk_filter(...)]`: the crate-wide interface filter.
    pub walk_filter: WalkFilter,
    /// The driver was invoked with a verbose flag (`-v` / `--verbose`).
    ///
    /// A warn-and-skip diagnostic for an item from another crate is silent by default: the user
    /// cannot act on a `std`/dependency method that the interface cannot represent. With this on,
    /// those diagnostics are emitted like local ones.
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_emit_policy: DefaultEmitPolicy::default(),
            use_doc_comment_tool_attrs: false,
            output: OutputConfig::default(),
            implicitly_emit_external_interfaces: false,
            max_walk_depth: 6,
            max_external_depth: 1,
            max_trait_impl_depth: 4,
            max_trait_interface_depth: 2,
            max_self_ref_depth: 1,
            walk_filter: WalkFilter::default(),
            verbose: false,
        }
    }
}
