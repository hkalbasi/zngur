//! Binary entry point for the `autozng-driver` executable.
//!
//! It behaves like `rustc`: it takes rustc arguments from the command line, appends
//! `--cfg autozng` (so `#[cfg_attr(autozng, autozng::...)]` tool attrs activate), then runs
//! [`analyze`] through the `RustcPublic` driver wrapper.
//!
//! It is intended to be invoked as `RUSTC_WORKSPACE_WRAPPER`, so the process must exit 0
//! whenever the *compiler* succeeded even if autozng had nothing to do: `Skipped` and
//! `Interrupted` (analysis-side) still map to `SUCCESS`, and only a failed compile is
//! `FAILURE`.

use std::process::ExitCode;

use rustc_public::CompilerError;

use crate::analyze::analyze;

pub fn run() -> ExitCode {
    let mut rustc_args: Vec<String> = std::env::args().collect();
    rustc_args.extend(["--cfg", "autozng"].iter().copied().map(Into::into));
    let result = crate::driver::RustcPublic::new(analyze).run(&rustc_args);
    match result {
        Ok(_) | Err(CompilerError::Skipped | CompilerError::Interrupted(_)) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
