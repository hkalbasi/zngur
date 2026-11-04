//! Entry point for the `cargo autozng` / `cargo-autozng` command.
//!
//! The flow mirrors `cargo-clippy`:
//!
//! 1. normalise `env::args` so `cargo autozng …` and `cargo-autozng …` behave identically;
//! 2. locate the [`autozng-driver`](../autozng_driver/index.html) executable;
//! 3. re-invoke cargo with the driver as `RUSTC_WORKSPACE_WRAPPER`, so a `.zng` interface file
//!    is emitted per workspace member.

use crate::args::{self, Command};
use crate::help;

/// Parse the command line, dispatch, and return the exit code.
pub fn run() -> Result<i32, String> {
    let raw_args: Vec<String> = std::env::args().collect();
    let invocation = args::parse(&raw_args)?;

    match invocation.command {
        Command::Help => {
            println!("{}", help::help_text());
            Ok(0)
        }
        Command::Version => {
            println!("{}", help::version_text());
            Ok(0)
        }
        Command::Generate => crate::generate::generate(&invocation.args),
    }
}
