//! Resolving the toolchain the driver needs.
//!
//! The driver embeds the compiler: it links `librustc_driver` from one exact nightly, so the
//! compiler forced for the wrapped build must be the one the driver was built against — not the
//! frontend's, and not the project's. The frontend learns which one from the driver's
//! `--version` handshake (see [`crate::launch::preflight`]) and resolves it here.
//!
//! Only the *managed* path (a rustup channel) is implemented; a provided toolchain root is
//! future work.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a resolved toolchain came from, which decides the environment the launch sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainSource {
    /// A channel resolved through rustup; `RUSTUP_TOOLCHAIN` is forced for the child.
    Managed { channel: String },
    /// The ambient environment; nothing is forced.
    Ambient,
}

/// A resolved toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// The `rustc` to probe with (the rustup proxy in managed mode).
    pub rustc: PathBuf,
    /// The `cargo` to drive.
    pub cargo: PathBuf,
    /// The sysroot to inject into the driver's rustc arguments.
    pub sysroot: PathBuf,
    /// The directory to prepend to the OS loader path so the driver can load `librustc_driver`.
    pub dylib_dir: PathBuf,
    /// The `rustc --version` line: the toolchain's identity.
    pub identity: String,
    /// Where it came from.
    pub source: ToolchainSource,
}

/// The directory that must be on the OS loader path so the driver can load `librustc_driver`.
///
/// Windows keeps `rustc_driver-<hash>.dll` next to `rustc.exe` in `bin`; Unix keeps
/// `librustc_driver-<hash>.so` in `lib`. The whole directory is needed, not one file: the driver
/// library itself depends on the LLVM shared library in the same directory.
pub fn dylib_dir(sysroot: &Path) -> PathBuf {
    if cfg!(windows) {
        sysroot.join("bin")
    } else {
        sysroot.join("lib")
    }
}

impl Toolchain {
    /// Derive a toolchain from a sysroot and an identity, given the channel that produced them.
    ///
    /// Pure, so the derivation is testable without running anything.
    pub fn derive(channel: Option<&str>, sysroot: PathBuf, identity: String) -> Toolchain {
        Toolchain {
            rustc: ambient_rustc(),
            cargo: ambient_cargo(),
            dylib_dir: dylib_dir(&sysroot),
            sysroot,
            identity,
            source: match channel {
                Some(channel) => ToolchainSource::Managed {
                    channel: channel.to_string(),
                },
                None => ToolchainSource::Ambient,
            },
        }
    }

    /// Resolve a channel through rustup (`RUSTUP_TOOLCHAIN=<channel>`).
    pub fn managed(channel: &str) -> Result<Toolchain, String> {
        let (sysroot, identity) = probe(Some(channel))?;
        Ok(Toolchain::derive(Some(channel), sysroot, identity))
    }

    /// Use the ambient environment, forcing nothing.
    pub fn ambient() -> Result<Toolchain, String> {
        let (sysroot, identity) = probe(None)?;
        Ok(Toolchain::derive(None, sysroot, identity))
    }

    /// The environment variables the wrapped cargo build must run under.
    pub fn launch_env(&self) -> Vec<(&'static str, OsString)> {
        match &self.source {
            ToolchainSource::Managed { channel } => {
                vec![("RUSTUP_TOOLCHAIN", OsString::from(channel))]
            }
            ToolchainSource::Ambient => Vec::new(),
        }
    }
}

/// The `cargo` to drive: cargo's own `CARGO` variable, or `cargo` on `PATH`.
pub fn ambient_cargo() -> PathBuf {
    env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

/// The `rustc` to probe with: `RUSTC` when set, else the rustup proxy on `PATH`.
pub fn ambient_rustc() -> PathBuf {
    env::var_os("RUSTC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rustc"))
}

/// Run `rustc --print sysroot` and `rustc --version`, optionally under a forced rustup channel.
fn probe(channel: Option<&str>) -> Result<(PathBuf, String), String> {
    let rustc = ambient_rustc();
    let sysroot = run_capture(&rustc, &["--print", "sysroot"], channel)?;
    let identity = run_capture(&rustc, &["--version"], channel)?;
    Ok((PathBuf::from(sysroot.trim()), identity.trim().to_string()))
}

/// Run `program args…`, returning stdout, with `RUSTUP_TOOLCHAIN` set when a channel is given.
fn run_capture(program: &Path, args: &[&str], channel: Option<&str>) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(channel) = channel {
        command.env("RUSTUP_TOOLCHAIN", channel);
    }

    let output = command.output().map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => format!(
            "`{}` was not found on `PATH`; the frontend needs it to locate the toolchain",
            program.display()
        ),
        _ => format!("failed to run `{}`: {err}", program.display()),
    })?;
    if !output.status.success() {
        return Err(format!(
            "`{} {}` failed:\n{}",
            program.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
