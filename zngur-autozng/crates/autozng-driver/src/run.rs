//! Binary entry point for the `autozng-driver` executable.
//!
//! It behaves like `rustc`: it takes rustc arguments from the command line, appends
//! `--cfg autozng` (so `#[cfg_attr(autozng, autozng::...)]` tool attrs activate), then runs
//! [`analyze`] through the `RustcPublic` driver wrapper.
//!
//! In addition to the rustc arguments it understands two autozng-specific flags, which are
//! stripped before the arguments reach `rustc`:
//!
//! - `--output-zng <path>` / `--output-zng=<path>`: write the generated `.zng` to `path`.
//! - `--out-dir <dir>` / `--out-dir=<dir>`: recorded for the default `.zng` location (the
//!   argument itself is still forwarded to `rustc`). rustc's `-o` is left untouched.
//!
//! When neither `--output-zng` nor `--out-dir` is given the `.zng` is named after the crate
//! and written to the current directory (rustc's default output directory).
//!
//! # Wrapper mode
//!
//! The driver is normally invoked as `RUSTC_WORKSPACE_WRAPPER`, where cargo passes the *real*
//! rustc path as the first argument ([`is_wrapper_mode`]). That argument is stripped so the
//! remaining arguments are the ordinary rustc arguments; leaving it in place makes rustc see
//! two input filenames. Because a wrapper receives nothing but rustc arguments, the output
//! location travels in the environment instead: `AUTOZNG_OUTPUT_DIR` names a directory (the
//! file is `<crate name>.zng` inside it) and `AUTOZNG_OUTPUT_ZNG` names an explicit file.
//!
//! Cargo also probes the wrapper with `-vV` and `--print=…` before and during a build
//! ([`is_info_query`]); the injected flags are skipped on those queries so the answers are the
//! real compiler's and cargo's cache is not poisoned.
//!
//! Outside wrapper mode a direct `<driver> --version` answers with the autozng identity
//! ([`crate::version`]) rather than rustc's, which is how `cargo-autozng` detects a driver and
//! learns which toolchain it needs.
//!
//! It is intended to be invoked as `RUSTC_WORKSPACE_WRAPPER`, so the process must exit 0
//! whenever the *compiler* succeeded even if autozng had nothing to do: `Skipped` and
//! `Interrupted` (analysis-side) still map to `SUCCESS`, and only a failed compile is
//! `FAILURE`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rustc_public::CompilerError;

use crate::analyze::analyze;
use crate::config::{self, Config};

pub fn run() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().collect();

    // A direct invocation asking for `--version` gets autozng's identity (the handshake
    // `cargo-autozng` reads). In wrapper mode `--version` belongs to the real compiler.
    if !is_wrapper_mode(&raw_args) && wants_own_version(&raw_args) {
        print!("{}", crate::version::handshake());
        return ExitCode::SUCCESS;
    }

    let (config, mut rustc_args) = parse_args(raw_args);

    // Cargo's information queries (`-vV`, `--print=…`) must be answered by the compiler
    // unchanged: an injected `--cfg autozng` would leak into the cached `--print=cfg` output (and
    // a query carrying unexpected flags poisons cargo's cache).
    if !is_info_query(&rustc_args) {
        rustc_args.extend(["--cfg".to_string(), "autozng".to_string()]);

        // Belt-and-braces: point the compiler at the sysroot `cargo-autozng` resolved. The
        // sysroot already follows the `librustc_driver` the driver loaded, so this is a second
        // route to the same answer, and an explicit user `--sysroot` always wins.
        if let Some(sysroot) = std::env::var_os(config::ENV_SYSROOT)
            && !has_sysroot(&rustc_args)
        {
            rustc_args.push("--sysroot".to_string());
            rustc_args.push(sysroot.to_string_lossy().into_owned());
        }
    }

    let result = crate::driver::RustcPublic::new(analyze, config).run(&rustc_args);
    match result {
        Ok(_) | Err(CompilerError::Skipped | CompilerError::Interrupted(_)) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Whether `args` is a wrapper invocation, i.e. cargo passed the real rustc as `argv[1]`.
///
/// The test is the *file stem* being `rustc`, so both `rustc` and `rustc.exe` match on any
/// platform. It is deliberately not "is an executable that exists": a wrapper receives source
/// paths too, and a rule that guessed would strip a legitimate input filename.
fn is_wrapper_mode(args: &[String]) -> bool {
    args.get(1)
        .map(Path::new)
        .and_then(Path::file_stem)
        .and_then(OsStr::to_str)
        == Some("rustc")
}

/// Whether the direct invocation asked for the autozng version handshake.
fn wants_own_version(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
}

/// Whether `args` is one of cargo's information queries.
///
/// Cargo learns about the compiler with `rustc -vV` and `rustc --print …` (both the split
/// `--print cfg` and joined `--print=cfg` forms). Those answers are cached, so autozng must not
/// add its own flags to them.
fn is_info_query(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "-vV" || arg == "--print" || arg.starts_with("--print="))
}

/// Whether a `--sysroot` is already present, in either the split or the joined form.
fn has_sysroot(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--sysroot" || arg.starts_with("--sysroot="))
}

/// Split autozng's own flags out of the incoming rustc argument list.
///
/// A leading wrapper argument ([`is_wrapper_mode`]) is dropped. The returned argument list is
/// what gets handed to `rustc_driver::run_compiler`; only `--output-zng` is removed (it is not a
/// rustc flag). `--out-dir` is forwarded unchanged but also recorded on the [`Config`] so the
/// render step can honour it. The output location starts from `AUTOZNG_OUTPUT_DIR` /
/// `AUTOZNG_OUTPUT_ZNG` and an explicit command-line `--output-zng` overrides the latter.
fn parse_args(args: impl IntoIterator<Item = String>) -> (Config, Vec<String>) {
    let mut config = Config::default();

    // The out-of-band channel a wrapper invocation has to use (see `config::ENV_OUTPUT_DIR`).
    if let Some(dir) = std::env::var_os(config::ENV_OUTPUT_DIR) {
        config.output.output_dir = Some(PathBuf::from(dir));
    }
    if let Some(path) = std::env::var_os(config::ENV_OUTPUT_ZNG) {
        config.output.output_zng = Some(PathBuf::from(path));
    }

    let mut args: Vec<String> = args.into_iter().collect();
    if is_wrapper_mode(&args) {
        args.remove(1);
    }

    let mut rustc_args = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--output-zng=") {
            config.output.output_zng = Some(PathBuf::from(value));
            continue;
        }
        if arg == "--output-zng" {
            if let Some(value) = args.next() {
                config.output.output_zng = Some(PathBuf::from(value));
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--out-dir=") {
            config.output.out_dir = Some(PathBuf::from(value));
            rustc_args.push(arg);
            continue;
        }
        if arg == "--out-dir" {
            if let Some(value) = args.next() {
                config.output.out_dir = Some(PathBuf::from(&value));
                rustc_args.push(arg);
                rustc_args.push(value);
            } else {
                rustc_args.push(arg);
            }
            continue;
        }
        rustc_args.push(arg);
    }

    (config, rustc_args)
}
