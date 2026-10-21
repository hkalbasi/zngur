//! Bakes the pinned toolchain channel into the frontend.
//!
//! The channel is the *known-good* nightly this workspace builds drivers against
//! (`rust-toolchain.toml`), so no Rust source names a nightly date and the constant cannot
//! drift from the file. It is the default `setup` uses and the fallback candidate the preflight
//! tries when the driver cannot report its own channel — it is *not* a pin: the toolchain the
//! run uses is the driver's (see the crate docs).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let toolchain_file = find_toolchain_file();
    println!("cargo::rerun-if-changed={}", toolchain_file.display());

    let contents = fs::read_to_string(&toolchain_file)
        .unwrap_or_else(|err| panic!("failed to read `{}`: {err}", toolchain_file.display()));
    println!(
        "cargo::rustc-env=AUTOZNG_KNOWN_GOOD_TOOLCHAIN={}",
        parse_channel(&contents)
    );
}

/// Walk up from the crate directory to the workspace's `rust-toolchain.toml`.
///
/// The same script then serves the workspace layout and the published-crate layout, where each
/// crate ships its own copy (cargo runs a publish verification build under `target/package/`).
fn find_toolchain_file() -> PathBuf {
    let mut dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    loop {
        let candidate = dir.join("rust-toolchain.toml");
        if candidate.is_file() {
            return candidate;
        }
        if !dir.pop() {
            panic!("no `rust-toolchain.toml` found above `CARGO_MANIFEST_DIR`");
        }
    }
}

/// Read the `channel` field with a deliberately minimal scan, so the build script needs no
/// dependency for one well-known field in a file we own.
fn parse_channel(contents: &str) -> String {
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("channel") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches(['"', '\'']);
        if !value.is_empty() {
            return value.to_string();
        }
    }
    panic!("`channel` not found in `rust-toolchain.toml`");
}
