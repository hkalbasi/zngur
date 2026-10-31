//! The OS dynamic-library search path.
//!
//! The driver links `rustc_driver` dynamically, so the toolchain's library directory must be on
//! the loader path whenever the driver runs — both when cargo spawns it and when the preflight
//! runs it as a load test.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The environment variable the OS loader reads for extra library directories.
#[cfg(target_os = "macos")]
pub const DYLIB_PATH_VAR: &str = "DYLD_FALLBACK_LIBRARY_PATH";
/// The environment variable the OS loader reads for extra library directories.
#[cfg(target_os = "windows")]
pub const DYLIB_PATH_VAR: &str = "PATH";
/// The environment variable the OS loader reads for extra library directories.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const DYLIB_PATH_VAR: &str = "LD_LIBRARY_PATH";

/// `dir` prepended to `existing`, as the platform's path list.
///
/// A whole directory is prepended, never a single file: `librustc_driver` itself depends on the
/// LLVM shared library that lives beside it, so the loader needs the directory to search.
pub fn prepend(dir: &Path, existing: Option<&OsStr>) -> OsString {
    let Some(existing) = existing.filter(|value| !value.is_empty()) else {
        return dir.as_os_str().to_os_string();
    };

    let mut entries: Vec<PathBuf> = vec![dir.to_path_buf()];
    entries.extend(env::split_paths(existing));
    env::join_paths(entries).expect("a toolchain directory is always a valid path entry")
}
