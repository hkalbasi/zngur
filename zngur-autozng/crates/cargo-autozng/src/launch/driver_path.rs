//! Locating the `autozng-driver` executable.
//!
//! The driver cannot be `cargo install`ed (it needs `rustc_private`), so alongside the usual
//! sibling lookup the frontend also looks in the autozng workspace's target directory, which is
//! where a checkout leaves it. Building the driver is deliberately **not** part of this search:
//! the generation command must never do slow, stateful work, so a missing driver is reported
//! with a pointer at `cargo autozng setup` rather than silently compiled.

use std::env;
use std::path::{Path, PathBuf};

use crate::config;

/// The driver's file name, including the platform's executable suffix.
pub fn driver_file_name() -> String {
    format!("{}{}", config::DRIVER_BIN, env::consts::EXE_SUFFIX)
}

/// The autozng workspace root, from `AUTOZNG_WORKSPACE` or the frontend's own build location.
pub fn workspace_root() -> PathBuf {
    env::var_os(config::ENV_WORKSPACE)
        .map(PathBuf::from)
        .unwrap_or_else(default_workspace_root)
}

/// The autozng workspace root inferred from where the frontend was built.
///
/// `cargo build` in the workspace produces `target/{debug,release}/cargo-autozng` with the
/// driver beside it, so in a checkout the sibling lookup already finds the driver; this is the
/// fallback for a frontend that was copied out of `target/` (`crates/cargo-autozng` → `../..`).
fn default_workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The locations searched for the driver, in priority order.
///
/// * an explicit `AUTOZNG_DRIVER` (also what a test harness sets);
/// * a sibling of the running frontend;
/// * the autozng workspace's `target/release`, then `target/debug` (source mode);
/// * the bare name, left for the OS to resolve on `PATH`.
///
/// Purely a function of its inputs, so the ordering is testable without a filesystem.
pub fn search_order(
    env_driver: Option<PathBuf>,
    current_exe: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
) -> Vec<PathBuf> {
    let file_name = driver_file_name();
    let mut candidates = Vec::new();

    if let Some(driver) = env_driver {
        candidates.push(driver);
    }
    if let Some(dir) = current_exe.as_deref().and_then(Path::parent) {
        candidates.push(dir.join(&file_name));
    }
    if let Some(root) = workspace_root {
        candidates.push(root.join("target").join("release").join(&file_name));
        candidates.push(root.join("target").join("debug").join(&file_name));
    }
    candidates.push(PathBuf::from(&file_name));

    candidates
}

/// The first candidate that exists, or `None`.
pub fn locate_in(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

/// The driver to use.
///
/// The first existing candidate, or the bare name so the OS can still resolve it on `PATH` and
/// the spawn reports a clear "not found" when it cannot.
pub fn locate() -> PathBuf {
    let candidates = search_order(
        env::var_os(config::ENV_DRIVER).map(PathBuf::from),
        env::current_exe().ok(),
        Some(workspace_root()),
    );
    locate_in(&candidates).unwrap_or_else(|| PathBuf::from(driver_file_name()))
}
