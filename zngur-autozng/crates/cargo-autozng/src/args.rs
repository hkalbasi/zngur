//! Command-line parsing and argument normalisation.
//!
//! The frontend forwards every argument it does not recognise to `cargo check`, so the parsing
//! is hand-rolled: `clap`'s pass-through handling fights with that requirement, and the surface
//! is small enough that a table is clearer than a framework.

use std::path::PathBuf;

use crate::config;

/// The command the frontend was asked to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Generate the `.zng` files (the default command).
    Generate,
    /// Print the help text.
    Help,
    /// Print the frontend version.
    Version,
}

/// The parsed options of a [`Command::Generate`] invocation.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Path to the `autozng-driver` executable (`--driver`), overriding discovery.
    pub driver: Option<PathBuf>,
    /// Directory the `.zng` files are written to (`--output-dir` / `-o`).
    pub output_dir: Option<PathBuf>,
    /// Explicit file for a single crate (`--output-zng`).
    pub output_zng: Option<PathBuf>,
    /// Toolchain root to use instead of a rustup channel (`--toolchain`).
    pub toolchain: Option<PathBuf>,
    /// Target directory for the wrapped build (`--target-dir`).
    pub target_dir: Option<PathBuf>,
    /// Use the `release` profile (`--release`).
    pub release: bool,
    /// Print the resolved driver and command without running anything (`--dry-run`).
    pub dry_run: bool,
    /// Echo the discovery steps and the cargo command (`-v` / `--verbose`).
    pub verbose: bool,
    /// Everything else, forwarded to `cargo check` verbatim.
    pub cargo_args: Vec<String>,
}

/// A parsed command line: what to do, and (for [`Command::Generate`]) with which options.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    pub args: Args,
}

impl Invocation {
    fn command(command: Command) -> Self {
        Invocation {
            command,
            args: Args::default(),
        }
    }
}

/// Drop the program name and, if present, the cargo-inserted subcommand token.
///
/// `cargo autozng <args>` invokes `cargo-autozng autozng <args>`, while a direct
/// `cargo-autozng <args>` has no such token, so both forms normalise to the same `<args>`. Only
/// a *leading* token is stripped: a later argument that happens to equal the subcommand name
/// survives.
pub fn strip_subcommand(
    raw_args: impl IntoIterator<Item = String>,
    subcommand: &str,
) -> Vec<String> {
    let mut raw_args = raw_args.into_iter();
    let _program = raw_args.next();
    let mut args: Vec<String> = raw_args.collect();
    if args.first().is_some_and(|first| first == subcommand) {
        args.remove(0);
    }
    args
}

/// Parse `raw_args` (including the program name).
pub fn parse(raw_args: &[String]) -> Result<Invocation, String> {
    let args = strip_subcommand(raw_args.iter().cloned(), config::CARGO_SUBCOMMAND);

    // No arguments at all, or a leading help request, is a friendly overview rather than an
    // error.
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => return Ok(Invocation::command(Command::Help)),
        Some("-V" | "--version") => return Ok(Invocation::command(Command::Version)),
        Some(subcommand @ ("setup" | "update")) => {
            return Err(format!(
                "the `{subcommand}` subcommand is not implemented yet"
            ));
        }
        Some(_) => {}
    }

    let mut options = Args::default();
    let mut iter = args.into_iter().peekable();
    if iter.peek().map(String::as_str) == Some("generate") {
        iter.next();
    }

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Invocation::command(Command::Help)),
            "-V" | "--version" => return Ok(Invocation::command(Command::Version)),
            "--release" => options.release = true,
            "--dry-run" => options.dry_run = true,
            "-v" | "--verbose" => options.verbose = true,
            "--driver" => options.driver = Some(PathBuf::from(take_value(&mut iter, "--driver")?)),
            "--output-dir" | "-o" => {
                options.output_dir = Some(PathBuf::from(take_value(&mut iter, "--output-dir")?));
            }
            "--output-zng" => {
                options.output_zng = Some(PathBuf::from(take_value(&mut iter, "--output-zng")?));
            }
            "--toolchain" => {
                options.toolchain = Some(PathBuf::from(take_value(&mut iter, "--toolchain")?));
            }
            "--target-dir" => {
                options.target_dir = Some(PathBuf::from(take_value(&mut iter, "--target-dir")?));
            }
            // Everything after `--` belongs to cargo untouched, including anything that looks
            // like one of our flags.
            "--" => {
                options.cargo_args.push(arg);
                options.cargo_args.extend(iter);
                break;
            }
            _ => {
                if !parse_joined(&arg, &mut options) {
                    options.cargo_args.push(arg);
                }
            }
        }
    }

    Ok(Invocation {
        command: Command::Generate,
        args: options,
    })
}

/// Consume a `--flag=value` argument, returning whether it was recognised.
fn parse_joined(arg: &str, options: &mut Args) -> bool {
    let Some((flag, value)) = arg.split_once('=') else {
        return false;
    };
    let value = PathBuf::from(value);
    match flag {
        "--driver" => options.driver = Some(value),
        "--output-dir" => options.output_dir = Some(value),
        "--output-zng" => options.output_zng = Some(value),
        "--toolchain" => options.toolchain = Some(value),
        "--target-dir" => options.target_dir = Some(value),
        _ => return false,
    }
    true
}

/// Take the value that follows a flag, or report the flag is missing it.
fn take_value(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("`{flag}` requires a value"))
}
