use std::path::{Path, PathBuf};

use clap::Parser;

mod autozng;
mod ci;
mod format_book;
mod format_templates;
mod test;

/// Repository root, resolved from this crate's manifest so paths don't depend
/// on the directory `cargo xtask` was invoked from.
pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the repository")
        .to_owned()
}

#[derive(Parser)]
enum Command {
    CI {
        #[arg(long)]
        fix: bool,
        /// Run only the `zngur-autozng` tests.
        #[arg(long)]
        only_autozng: bool,
        /// Fail instead of skipping `zngur-autozng` tests when no toolchain resolves.
        #[arg(long)]
        require_autozng: bool,
        /// Report autozng bridge generation/compile failures as skipped instead of failed.
        #[arg(long)]
        allow_autozng_bridge_failures: bool,
    },
    /// Run the end-to-end tests and examples.
    Test {
        /// Glob matched against the path relative to the repository root
        /// (e.g. `tests/parser/*` or `examples/simple/*`). Runs everything when omitted.
        filter: Option<String>,
        /// Read glob filters from this file (one per line; blank lines and
        /// lines starting with `#` are ignored).
        #[arg(long, conflicts_with = "filter")]
        filter_file: Option<std::path::PathBuf>,
        /// Write failing test paths to this file (default: `fail_list.txt` in
        /// the repository root). Consume later with `--filter-file`.
        #[arg(long)]
        fail_list: Option<std::path::PathBuf>,
        /// Update snapshots instead of comparing against them.
        #[arg(long)]
        fix: bool,
        /// Print every passing test instead of a progress dot.
        #[arg(short, long)]
        verbose: bool,
        /// Run only the `zngur-autozng` tests.
        #[arg(long)]
        only_autozng: bool,
        /// Fail instead of skipping `zngur-autozng` tests when no toolchain resolves.
        #[arg(long)]
        require_autozng: bool,
        /// Report autozng bridge generation/compile failures as skipped instead of failed.
        #[arg(long)]
        allow_autozng_bridge_failures: bool,
    },
    FormatBook {
        #[arg(long)]
        fix: bool,
    },
    FormatTemplates {
        #[arg(long)]
        fix: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cmd = Command::parse();
    match cmd {
        Command::CI {
            fix,
            only_autozng,
            require_autozng,
            allow_autozng_bridge_failures,
        } => ci::main(
            fix,
            only_autozng,
            require_autozng,
            allow_autozng_bridge_failures,
        ),
        Command::FormatBook { fix } => format_book::main(fix),
        Command::FormatTemplates { fix } => format_templates::main(fix),
        Command::Test {
            filter,
            filter_file,
            fail_list,
            fix,
            verbose,
            only_autozng,
            require_autozng,
            allow_autozng_bridge_failures,
        } => test::main_with(
            fix,
            filter,
            filter_file,
            fail_list,
            verbose,
            only_autozng,
            require_autozng,
            allow_autozng_bridge_failures,
        ),
    }
}
