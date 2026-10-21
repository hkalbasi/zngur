//! The preflight: ask the driver what it needs, then verify it can be used.
//!
//! Read-only and fast — it never installs or builds anything. On failure it stops with a message
//! naming the fix, because provisioning the driver is a separate, deliberate step.
//!
//! The order follows from the driver owning the compiler requirement: the driver is asked what
//! it needs *before* anything is provided to it.
//!
//! 1. **Does it run?** [`probe`] runs `<driver> --version` under a candidate toolchain's loader
//!    path. This is simultaneously a *load test* (a driver built against another nightly fails to
//!    launch, because the loader cannot find its `librustc_driver`) and the *handshake*.
//! 2. **Does it match, and can the frontend use it?** [`evaluate`] compares the toolchain's
//!    `rustc --version` with the driver's `built-against-rustc` (hard), the driver's version with
//!    the frontend's (a warning only), and its capability set with what the frontend needs (hard).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::config;
use crate::launch::dylib;
use crate::toolchain::Toolchain;

/// The capabilities the frontend cannot work without.
///
/// The frontend directs the driver's output through `AUTOZNG_OUTPUT_DIR`, so a driver that does
/// not advertise `output-dir-env` would write its `.zng` somewhere unexpected. Everything else
/// (the `--sysroot` hint, for instance) is optional.
pub const REQUIRED_CAPABILITIES: &[&str] = &["output-dir-env"];

/// The driver's answer to `--version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverHandshake {
    /// The driver's own version.
    pub tool_version: String,
    /// The channel the driver was meant to be built with.
    pub pinned_toolchain: String,
    /// The compiler that actually compiled the driver.
    pub built_against_rustc: String,
    /// The frontend-facing features the driver implements.
    pub capabilities: BTreeSet<String>,
}

/// The verdict of the preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Everything matches.
    Ok,
    /// Non-fatal problems worth reporting (a version drift).
    Warn(Vec<String>),
    /// Fatal problems: the driver cannot be used with this toolchain.
    Fail(Vec<String>),
}

/// Parse the driver's `--version` handshake.
///
/// Returns `None` when the output is not an autozng handshake (a foreign or older binary), so
/// the preflight can report it rather than misread it.
pub fn parse_handshake(stdout: &str) -> Option<DriverHandshake> {
    let mut lines = stdout.lines();
    let tool_version = lines
        .next()?
        .trim()
        .strip_prefix(&format!("{} ", config::DRIVER_BIN))?
        .trim()
        .to_string();
    if tool_version.is_empty() {
        return None;
    }

    let mut pinned_toolchain = None;
    let mut built_against_rustc = None;
    let mut capabilities = None;
    for line in lines {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("pinned-toolchain:") {
            pinned_toolchain = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("built-against-rustc:") {
            built_against_rustc = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("capabilities:") {
            capabilities = Some(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|capability| !capability.is_empty())
                    .map(str::to_string)
                    .collect(),
            );
        }
    }

    Some(DriverHandshake {
        tool_version,
        pinned_toolchain: pinned_toolchain?,
        built_against_rustc: built_against_rustc?,
        capabilities: capabilities?,
    })
}

/// Judge a handshake against the toolchain the frontend will use.
///
/// * the toolchain's `rustc --version` must equal the driver's `built-against-rustc` — **hard**,
///   because a driver cannot load `librustc_driver` from a different compiler;
/// * every capability in `required` must be advertised — **hard**;
/// * the driver's `tool_version` need only match the frontend's version for a **warning**: the
///   two are separate programs with a small, stable interface, so a version difference is a hint
///   that the pair is out of step, not proof that anything is broken.
pub fn evaluate(
    handshake: &DriverHandshake,
    toolchain_rustc_version: &str,
    frontend_version: &str,
    required: &[&str],
) -> Verdict {
    if handshake.built_against_rustc.trim() != toolchain_rustc_version.trim() {
        // The driver cannot run at all against a different compiler; do not pile on.
        return Verdict::Fail(vec![format!(
            "the installed autozng-driver was built against `{}`, but the selected toolchain \
             provides `{}`",
            handshake.built_against_rustc, toolchain_rustc_version,
        )]);
    }

    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|capability| !handshake.capabilities.contains(*capability))
        .collect();
    if !missing.is_empty() {
        return Verdict::Fail(vec![format!(
            "the installed autozng-driver is missing the required `{}` capability; rebuild it \
             with `cargo autozng setup`",
            missing.join("`, `"),
        )]);
    }

    let mut warnings = Vec::new();
    if handshake.tool_version != frontend_version {
        warnings.push(format!(
            "the installed autozng-driver is version {}, but this cargo-autozng is {} — \
             continuing, since the driver supports every feature this frontend needs",
            handshake.tool_version, frontend_version,
        ));
    }

    if warnings.is_empty() {
        Verdict::Ok
    } else {
        Verdict::Warn(warnings)
    }
}

/// Run `<driver> --version` under `toolchain`'s loader path and read its handshake.
pub fn probe(driver: &Path, toolchain: &Toolchain) -> Result<DriverHandshake, String> {
    let mut command = Command::new(driver);
    command.arg("--version").env(
        dylib::DYLIB_PATH_VAR,
        dylib::prepend(
            &toolchain.dylib_dir,
            std::env::var_os(dylib::DYLIB_PATH_VAR).as_deref(),
        ),
    );

    let output = command.output().map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => format!(
            "the autozng-driver executable `{}` was not found; build it in the autozng \
             workspace (`cargo build -p autozng-driver`) or point `--driver` at one",
            driver.display()
        ),
        _ => format!(
            "failed to run the autozng-driver `{}`: {err}",
            driver.display()
        ),
    })?;

    if !output.status.success() {
        return Err(format!(
            "the autozng-driver could not start ({}) — its toolchain is probably not installed:\n{}",
            exit_description(output.status),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_handshake(stdout.trim()).ok_or_else(|| {
        format!(
            "the autozng-driver did not answer `--version` with an autozng handshake:\n{stdout}"
        )
    })
}

/// How to describe a non-zero exit status to a human.
fn exit_description(status: ExitStatus) -> String {
    match status.code() {
        // On Windows an unloadable `librustc_driver` shows up as `STATUS_DLL_NOT_FOUND`
        // (`-1073741515`), so the raw code is the most useful thing to print.
        Some(code) => format!("exit code {code}"),
        None => "terminated by a signal".to_string(),
    }
}

/// How the frontend prepares to launch cargo.
#[derive(Debug, Clone)]
pub struct PrepareOptions {
    /// `AUTOZNG_NO_MANAGE`: skip the preflight and the toolchain forcing entirely.
    pub no_manage: bool,
    /// `AUTOZNG_TOOLCHAIN`: override the channel resolved through rustup.
    pub channel: Option<String>,
    /// Echo the steps taken.
    pub verbose: bool,
}

/// A usable toolchain plus what the preflight learned about the driver.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The toolchain the wrapped build runs under.
    pub toolchain: Toolchain,
    /// The driver's handshake, or `None` in ambient mode (no preflight ran).
    pub handshake: Option<DriverHandshake>,
    /// Non-fatal preflight findings to report.
    pub warnings: Vec<String>,
}

impl Prepared {
    /// Whether the driver advertises `capability`.
    ///
    /// Ambient mode (no handshake) trusts the environment and reports support.
    pub fn supports(&self, capability: &str) -> bool {
        self.handshake
            .as_ref()
            .is_none_or(|handshake| handshake.capabilities.contains(capability))
    }
}

/// Ask the driver what it needs and resolve a matching toolchain.
///
/// In managed mode the candidate channels are `AUTOZNG_TOOLCHAIN` (when set) and then the
/// frontend's known-good channel; each is tried until the driver starts and answers, and the one
/// that works is verified against the driver's baked identity. In ambient mode the preflight is
/// skipped and nothing is forced.
pub fn prepare(driver: &Path, options: &PrepareOptions) -> Result<Prepared, String> {
    if options.no_manage {
        if options.verbose {
            eprintln!("cargo-autozng: `AUTOZNG_NO_MANAGE` is set; skipping the preflight");
        }
        return Ok(Prepared {
            toolchain: Toolchain::ambient()?,
            handshake: None,
            warnings: Vec::new(),
        });
    }

    let channels = candidate_channels(options.channel.as_deref());
    let mut attempts = Vec::new();

    for channel in &channels {
        if options.verbose {
            eprintln!("cargo-autozng: trying toolchain `{channel}`");
        }

        let toolchain = match Toolchain::managed(channel) {
            Ok(toolchain) => toolchain,
            Err(err) => {
                attempts.push(format!("`{channel}`: {err}"));
                continue;
            }
        };

        let handshake = match probe(driver, &toolchain) {
            Ok(handshake) => handshake,
            Err(err) => {
                attempts.push(format!("`{channel}`: {err}"));
                continue;
            }
        };

        match evaluate(
            &handshake,
            &toolchain.identity,
            env!("CARGO_PKG_VERSION"),
            REQUIRED_CAPABILITIES,
        ) {
            Verdict::Ok => {
                if options.verbose {
                    eprintln!("cargo-autozng: using `{channel}` ({})", toolchain.identity);
                }
                return Ok(Prepared {
                    toolchain,
                    handshake: Some(handshake),
                    warnings: Vec::new(),
                });
            }
            Verdict::Warn(warnings) => {
                return Ok(Prepared {
                    toolchain,
                    handshake: Some(handshake),
                    warnings,
                });
            }
            Verdict::Fail(failures) => {
                attempts.push(format!("`{channel}`: {}", failures.join("; ")));
            }
        }
    }

    Err(compose_failure(&attempts))
}

/// The channels to try, de-duplicated, in priority order: the override first, then the
/// frontend's known-good channel.
pub fn candidate_channels(override_channel: Option<&str>) -> Vec<String> {
    let mut channels = Vec::new();
    if let Some(channel) = override_channel {
        channels.push(channel.to_string());
    }
    channels.push(config::KNOWN_GOOD_TOOLCHAIN.to_string());
    channels.dedup();
    channels
}

/// The message shown when no candidate channel yields a usable driver.
fn compose_failure(attempts: &[String]) -> String {
    let mut message = String::from("no usable autozng-driver/toolchain pair was found.\n");
    for attempt in attempts {
        message.push_str(&format!("\n  {attempt}"));
    }
    message.push_str(&format!(
        "\n\nThe driver needs the exact nightly it was built against. For the in-tree driver that \
         is `{}` with the `rustc-dev` component; install it and build the driver with `cargo \
         build -p autozng-driver` in the zngur-autozng workspace.",
        config::KNOWN_GOOD_TOOLCHAIN,
    ));
    message
}
