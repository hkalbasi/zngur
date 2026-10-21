//! The isolated target directory for the wrapped build.
//!
//! Sharing the project's own target directory would make cargo rebuild the dependencies on every
//! alternation between a wrapped and an unwrapped run, because `RUSTC_WORKSPACE_WRAPPER` is part
//! of the build fingerprint. An isolated directory also gives the `.zng` files a natural,
//! predictable home.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::toolchain::Toolchain;

/// The directory name used inside the project's target directory.
pub const SUBDIR: &str = "autozng";

/// The default `.zng` output directory: `<project target>/autozng`, the isolated build home.
pub fn default_output_dir(project_target_dir: Option<&Path>) -> PathBuf {
    project_target_dir
        .map(|target| target.join(SUBDIR))
        .unwrap_or_else(|| PathBuf::from("target").join(SUBDIR))
}

/// The `--target-dir` to pass to cargo.
///
/// An explicit `--target-dir` wins; a set `CARGO_TARGET_DIR` means the frontend passes nothing
/// (cargo reads the variable itself); otherwise the isolated `<project target>/autozng` is used.
pub fn cargo_target_dir(
    explicit: Option<&Path>,
    cargo_target_dir_env: Option<&OsStr>,
    project_target_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(dir.to_path_buf());
    }
    if cargo_target_dir_env.is_some() {
        return None;
    }
    Some(default_output_dir(project_target_dir))
}

/// Ask cargo for the project's target directory, honouring a configured `build.target-dir`.
///
/// Runs `cargo metadata --no-deps` under the resolved toolchain (so rustup does not resolve the
/// project's toolchain file) and returns `None` when cargo cannot answer, letting the caller
/// fall back to a relative `target/autozng`.
pub fn project_target_dir(manifest_path: Option<&Path>, toolchain: &Toolchain) -> Option<PathBuf> {
    let mut command = Command::new(&toolchain.cargo);
    command.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(manifest) = manifest_path {
        command.arg("--manifest-path").arg(manifest);
    }
    for (key, value) in toolchain.launch_env() {
        command.env(key, value);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let json = String::from_utf8(output.stdout).ok()?;
    json_string_field(&json, "target_directory").map(PathBuf::from)
}

/// Extract a string field from cargo's metadata JSON.
///
/// Deliberately minimal (no `serde` dependency): the object is flat and machine-generated, so a
/// scan for the key followed by a JSON string is enough. Returns `None` rather than a wrong
/// answer on an unexpected shape, so the caller falls back to the relative default.
pub fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let rest = json.get(json.find(&needle)? + needle.len()..)?;
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();

    let mut value = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(value),
            '\\' => match chars.next()? {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                '/' => value.push('/'),
                'n' => value.push('\n'),
                't' => value.push('\t'),
                'r' => value.push('\r'),
                // A `\uXXXX` escape (or anything else) is not worth decoding for one field.
                _ => return None,
            },
            other => value.push(other),
        }
    }
    None
}
