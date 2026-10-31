//! The frontend's help and version texts.

use crate::config;

/// The `--version` line.
pub fn version_text() -> String {
    format!("{} {}", config::FRONTEND_BIN, env!("CARGO_PKG_VERSION"))
}

/// The `--help` text.
pub fn help_text() -> String {
    "\
cargo-autozng — generate Zngur `.zng` interface files for a Rust workspace

USAGE:
    cargo autozng [OPTIONS] [CARGO ARGS...]
    cargo-autozng [OPTIONS] [CARGO ARGS...]

Runs `cargo check` with `autozng-driver` installed as the rustc workspace wrapper, so every
workspace member is analysed and a `.zng` file is emitted per crate.

OPTIONS:
    -o, --output-dir <DIR>   Directory the `.zng` files are written to
                             [default: <target-dir>/autozng]
        --output-zng <FILE>  Write a single crate's `.zng` to FILE (errors if more than one
                             crate is built)
        --driver <PATH>      Path to the `autozng-driver` executable
        --toolchain <PATH>   Toolchain root to use instead of rustup (not implemented yet)
        --target-dir <DIR>   Target directory for the wrapped build
                             [default: <project target>/autozng]
        --release            Use the `release` profile
        --dry-run            Print the resolved driver and command, run nothing
    -v, --verbose            Echo the driver discovery steps and the cargo command
    -h, --help               Print this help
    -V, --version            Print the version

Everything else is forwarded to `cargo check` unchanged.
"
    .to_string()
}
