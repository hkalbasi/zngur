//! Building the wrapped `cargo check` command.
//!
//! The command is assembled as data ([`WrappedCommand`]) so it can be printed by `--dry-run`
//! and turned into a [`Command`] to run, without the two ever drifting apart.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config;
use crate::launch::dylib;
use crate::launch::preflight::Prepared;

/// Everything the wrapped command needs.
pub struct Launch<'a> {
    /// The preflighted toolchain and driver capabilities.
    pub prepared: &'a Prepared,
    /// The driver executable, installed as `RUSTC_WORKSPACE_WRAPPER`.
    pub driver: &'a Path,
    /// The directory the driver writes the `.zng` files to.
    pub output_dir: &'a Path,
    /// An explicit `.zng` file for a single crate, if the user asked for one.
    pub output_zng: Option<&'a Path>,
    /// The `--target-dir` to pass, or `None` to let cargo decide (a set `CARGO_TARGET_DIR`).
    pub target_dir: Option<&'a Path>,
    /// Arguments forwarded to `cargo check`.
    pub cargo_args: &'a [String],
    /// Whether to build with the `release` profile.
    pub release: bool,
}

/// A `cargo check` invocation, held as data until it is run or printed.
pub struct WrappedCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub envs: Vec<(OsString, OsString)>,
}

impl WrappedCommand {
    /// Assemble the wrapped `cargo check`.
    pub fn build(launch: &Launch<'_>) -> WrappedCommand {
        let toolchain = &launch.prepared.toolchain;

        let mut args: Vec<OsString> = vec!["check".into()];
        if let Some(target_dir) = launch.target_dir {
            args.push("--target-dir".into());
            args.push(target_dir.as_os_str().to_os_string());
        }
        if launch.release {
            args.push("--release".into());
        }
        args.extend(launch.cargo_args.iter().map(OsString::from));

        let mut envs: Vec<(OsString, OsString)> = vec![
            (
                "RUSTC_WORKSPACE_WRAPPER".into(),
                launch.driver.as_os_str().to_os_string(),
            ),
            (
                config::ENV_OUTPUT_DIR.into(),
                launch.output_dir.as_os_str().to_os_string(),
            ),
        ];
        if let Some(file) = launch.output_zng {
            envs.push((
                config::ENV_OUTPUT_ZNG.into(),
                file.as_os_str().to_os_string(),
            ));
        }
        // The sysroot hint, only when the driver advertises that it understands it.
        if launch.prepared.supports("sysroot-env") {
            envs.push((
                config::ENV_SYSROOT.into(),
                toolchain.sysroot.as_os_str().to_os_string(),
            ));
        }
        // Toolchain forcing (managed mode); empty in ambient mode.
        envs.extend(
            toolchain
                .launch_env()
                .into_iter()
                .map(|(key, value)| (OsString::from(key), value)),
        );
        // The loader path the driver needs to find `librustc_driver`.
        envs.push((
            dylib::DYLIB_PATH_VAR.into(),
            dylib::prepend(
                &toolchain.dylib_dir,
                std::env::var_os(dylib::DYLIB_PATH_VAR).as_deref(),
            ),
        ));

        WrappedCommand {
            program: toolchain.cargo.clone(),
            args,
            envs,
        }
    }

    /// Turn the description into a runnable [`Command`].
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command.envs(self.envs.iter().map(|(key, value)| (key, value)));
        command
    }

    /// A human-readable rendering, for `--dry-run`.
    pub fn describe(&self) -> String {
        let mut text = self.program.to_string_lossy().into_owned();
        for arg in &self.args {
            text.push(' ');
            text.push_str(&arg.to_string_lossy());
        }
        for (key, value) in &self.envs {
            text.push_str(&format!(
                "\n  {}={}",
                key.to_string_lossy(),
                value.to_string_lossy()
            ));
        }
        text
    }
}
