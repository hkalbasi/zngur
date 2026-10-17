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
