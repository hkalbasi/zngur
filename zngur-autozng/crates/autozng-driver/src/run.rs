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
//! It is intended to be invoked as `RUSTC_WORKSPACE_WRAPPER`, so the process must exit 0
//! whenever the *compiler* succeeded even if autozng had nothing to do: `Skipped` and
//! `Interrupted` (analysis-side) still map to `SUCCESS`, and only a failed compile is
//! `FAILURE`.

use std::path::PathBuf;
use std::process::ExitCode;

use rustc_public::CompilerError;

use crate::analyze::analyze;
use crate::config::Config;

pub fn run() -> ExitCode {
    let (config, mut rustc_args) = parse_args(std::env::args());
    rustc_args.extend(["--cfg".to_string(), "autozng".to_string()]);
    let result = crate::driver::RustcPublic::new(analyze, config).run(&rustc_args);
    match result {
        Ok(_) | Err(CompilerError::Skipped | CompilerError::Interrupted(_)) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Split autozng's own flags out of the incoming rustc argument list.
///
/// The returned argument list is what gets handed to `rustc_driver::run_compiler`; only
/// `--output-zng` is removed (it is not a rustc flag). `--out-dir` is forwarded unchanged
/// but also recorded on the [`Config`] so the render step can honour it.
fn parse_args(args: impl IntoIterator<Item = String>) -> (Config, Vec<String>) {
    let mut config = Config::default();
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
