#![feature(rustc_private)]

use std::process::ExitCode;

fn main() -> ExitCode {
    autozng_driver::run::run()
}
