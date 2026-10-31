//! Running the wrapped `cargo check` that produces the `.zng` files.
//!
//! The flow: locate the driver, preflight it (see [`crate::launch::preflight`]), work out where
//! the output and the isolated target directory go, then run cargo with the driver as the
//! workspace wrapper. Cargo's exit code is propagated verbatim, and its output is inherited
//! rather than parsed, so progress and diagnostics appear exactly as a plain `cargo check`.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::args::{self, Args};
use crate::config;
use crate::launch::preflight::{self, PrepareOptions};
use crate::launch::{command, driver_path, target_dir};

/// Generate the `.zng` files for the workspace, returning cargo's exit code.
pub fn generate(options: &Args) -> Result<i32, String> {
    if options.toolchain.is_some() {
        return Err(
            "the `--toolchain` flag (provided-toolchain mode) is not implemented yet; the \
             frontend resolves the driver's channel through rustup"
                .to_string(),
        );
    }

    let driver = options.driver.clone().unwrap_or_else(driver_path::locate);
    if options.verbose {
        eprintln!("cargo-autozng: driver: {}", driver.display());
    }

    let prepared = preflight::prepare(
        &driver,
        &PrepareOptions {
            no_manage: env::var_os(config::ENV_NO_MANAGE).is_some(),
            channel: env::var(config::ENV_TOOLCHAIN).ok(),
            verbose: options.verbose,
        },
    )?;
    for warning in &prepared.warnings {
        eprintln!("cargo-autozng: warning: {warning}");
    }

    let manifest_path = args::manifest_path(&options.cargo_args);
    let project_target_dir =
        target_dir::project_target_dir(manifest_path.as_deref(), &prepared.toolchain);
    let output_dir = match &options.output_dir {
        Some(dir) => absolutize(dir.clone())?,
        None => absolutize(target_dir::default_output_dir(
            project_target_dir.as_deref(),
        ))?,
    };
    let cargo_target_dir = target_dir::cargo_target_dir(
        options.target_dir.as_deref(),
        env::var_os("CARGO_TARGET_DIR").as_deref(),
        project_target_dir.as_deref(),
    );

    // The driver writes `<crate name>.zng` here; a wrapper cannot create the directory itself
    // without guessing, so the frontend makes it first.
    fs::create_dir_all(&output_dir).map_err(|err| {
        format!(
            "failed to create the output directory `{}`: {err}",
            output_dir.display()
        )
    })?;

    let wrapped = command::WrappedCommand::build(&command::Launch {
        prepared: &prepared,
        driver: &driver,
        output_dir: &output_dir,
        output_zng: options.output_zng.as_deref(),
        target_dir: cargo_target_dir.as_deref(),
        cargo_args: &options.cargo_args,
        release: options.release,
    });

    if options.dry_run {
        println!("{}", wrapped.describe());
        return Ok(0);
    }

    let status = wrapped
        .to_command()
        .status()
        .map_err(|err| format!("failed to run `{}`: {err}", wrapped.program.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// Make `path` absolute against the frontend's working directory.
///
/// Cargo runs the wrapper with its own working directory, so a relative output directory would
/// otherwise resolve somewhere unexpected.
fn absolutize(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    env::current_dir()
        .map(|cwd| cwd.join(&path))
        .map_err(|err| format!("failed to read the current directory: {err}"))
}
