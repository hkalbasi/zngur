use clap::Parser;

mod ci;
mod format_book;
mod format_templates;
mod test;

#[derive(Parser)]
enum Command {
    CI {
        #[arg(long)]
        fix: bool,
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
        Command::CI { fix } => ci::main(fix),
        Command::FormatBook { fix } => format_book::main(fix),
        Command::FormatTemplates { fix } => format_templates::main(fix),
        Command::Test {
            filter,
            filter_file,
            fail_list,
            fix,
            verbose,
        } => test::main_with(fix, filter, filter_file, fail_list, verbose),
    }
}
