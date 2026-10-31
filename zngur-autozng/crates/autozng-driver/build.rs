//! Bakes the identity of the toolchain this driver was built against into the binary.
//!
//! `cargo-autozng` reads the identity back through the driver's `--version` handshake (see
//! `src/version.rs`) to decide which toolchain the driver needs. Two facts are baked:
//!
//! * the pinned channel, parsed from the workspace's `rust-toolchain.toml`, so no Rust source
//!   names a nightly date and the constant cannot drift from the file;
//! * the `rustc --version` line of the compiler that *actually* compiled the driver (cargo sets
//!   `RUSTC` for build scripts), so the frontend can require the same compiler rather than
//!   merely the same channel name.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let toolchain_file = find_toolchain_file();
    println!("cargo::rerun-if-changed={}", toolchain_file.display());
    println!("cargo::rerun-if-env-changed=RUSTC");

    let contents = fs::read_to_string(&toolchain_file)
        .unwrap_or_else(|err| panic!("failed to read `{}`: {err}", toolchain_file.display()));
    println!(
        "cargo::rustc-env=AUTOZNG_KNOWN_GOOD_TOOLCHAIN={}",
        parse_channel(&contents)
    );
    println!(
        "cargo::rustc-env=AUTOZNG_BUILT_AGAINST_RUSTC={}",
        rustc_version_line()
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

/// The `rustc --version` line of the compiler compiling this crate.
fn rustc_version_line() -> String {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(&rustc)
        .arg("--version")
        .output()
        .unwrap_or_else(|err| {
            panic!(
                "failed to run `{} --version`: {err}",
                rustc.to_string_lossy()
            )
        });
    assert!(
        output.status.success(),
        "`{} --version` failed:\n{}",
        rustc.to_string_lossy(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("`rustc --version` produced invalid UTF-8")
        .trim()
        .to_string()
}
