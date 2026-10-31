//! The pinned `zngur-autozng` toolchain.
//!
//! The nested workspace builds against a nightly with `rustc-dev`. Rustup is only used to
//! discover that toolchain: commands invoke its binaries directly, so a rustup-free machine
//! works when `AUTOZNG_TEST_SYSROOT` points at a toolchain root.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use xshell::{Cmd, Shell, cmd};

/// Overrides toolchain discovery with an explicit toolchain root.
pub const SYSROOT_ENV: &str = "AUTOZNG_TEST_SYSROOT";

/// Variables that would make the parent workspace's toolchain settings leak into the nested one.
const SCRUBBED_ENV: &[&str] = &[
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
];

/// The nested workspace inside the repository.
pub fn workspace_root(repo: &Path) -> PathBuf {
    repo.join("zngur-autozng")
}

/// The `[toolchain] channel` pin in `workspace/rust-toolchain.toml`.
pub fn pinned_channel(workspace: &Path) -> Result<String> {
    let file = workspace.join("rust-toolchain.toml");
    let contents =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    parse_channel(&contents).with_context(|| format!("no `channel` field in {}", file.display()))
}

fn parse_channel(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("channel") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches(['"', '\'']);
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// The install command shown when the pinned toolchain is missing.
pub fn install_hint(channel: &str) -> String {
    format!(
        concat!(
            "  install it with rustup:\n",
            "    rustup toolchain install {} --profile minimal \\\n",
            "      --component rustc-dev --component llvm-tools --component rustfmt\n",
            "  or point {} at an existing toolchain root (containing bin/cargo)",
        ),
        channel, SYSROOT_ENV
    )
}

/// Where a resolved toolchain came from.
#[derive(Clone, Debug)]
pub enum Source {
    /// `AUTOZNG_TEST_SYSROOT`
    Env,
    /// A rustup-managed channel.
    Rustup(String),
}

/// A usable `zngur-autozng` toolchain.
pub struct Toolchain {
    root: PathBuf,
    source: Source,
    rustc_version: String,
    has_rustfmt: bool,
    target_dir: PathBuf,
}

/// The outcome of toolchain discovery.
pub enum Resolution {
    Found(Toolchain),
    /// Why autozng work must be skipped, shown to the user as-is.
    Skipped(String),
}

/// The frontend and driver built from the nested workspace.
#[allow(dead_code)] // used by the autozng test modes
pub struct DriverBinaries {
    pub driver: PathBuf,
    pub frontend: PathBuf,
}

/// Find a toolchain, preferring `AUTOZNG_TEST_SYSROOT` and falling back to rustup.
pub fn resolve(workspace: &Path) -> Resolution {
    match resolve_inner(workspace) {
        Ok(toolchain) => Resolution::Found(toolchain),
        Err(reason) => Resolution::Skipped(reason),
    }
}

fn resolve_inner(workspace: &Path) -> std::result::Result<Toolchain, String> {
    if let Some(root) = std::env::var_os(SYSROOT_ENV) {
        return from_env(PathBuf::from(root), workspace);
    }
    let channel = pinned_channel(workspace).map_err(|err| format!("{err:#}"))?;
    from_rustup(&channel, workspace)
}

fn from_env(root: PathBuf, workspace: &Path) -> std::result::Result<Toolchain, String> {
    let prefix = format!("{SYSROOT_ENV}={}", root.display());
    if !root.is_dir() {
        return Err(format!("{prefix}: not a directory"));
    }
    let rustc = root.join("bin").join(exe("rustc"));
    let cargo = root.join("bin").join(exe("cargo"));
    let rustc_version = rustc_version(&rustc).map_err(|err| format!("{prefix}: {err:#}"))?;
    if !cargo.is_file() {
        return Err(format!("{prefix}: missing {}", cargo.display()));
    }
    if !rustc_dev_present(&root) {
        return Err(format!(
            "{prefix}: missing rustc-dev ({})",
            rustc_dev_detail(&root)
        ));
    }
    Ok(Toolchain {
        has_rustfmt: rustfmt_present(&root),
        target_dir: target_dir(workspace),
        root,
        source: Source::Env,
        rustc_version,
    })
}

fn from_rustup(channel: &str, workspace: &Path) -> std::result::Result<Toolchain, String> {
    let no_toolchain = |detail: &str| no_toolchain_message(channel, detail);

    let installed = rustup_output(&["toolchain", "list"])
        .map_err(|err| no_toolchain(&format!("rustup is not available: {err:#}")))?;
    if !installed
        .lines()
        .any(|line| toolchain_matches(line, channel))
    {
        return Err(no_toolchain(&format!(
            "toolchain `{channel}` is not installed"
        )));
    }

    let components = rustup_output(&["component", "list", "--toolchain", channel, "--installed"])
        .map_err(|err| no_toolchain(&format!("{err:#}")))?;
    if !components.lines().any(|line| line.starts_with("rustc-dev")) {
        return Err(no_toolchain(&format!(
            "`rustc-dev` is not installed for `{channel}`"
        )));
    }

    let sysroot = rustup_output(&["run", channel, "rustc", "--print", "sysroot"])
        .map_err(|err| no_toolchain(&format!("{err:#}")))?;
    let root = PathBuf::from(sysroot.trim());
    let rustc_version = rustc_version(&root.join("bin").join(exe("rustc")))
        .map_err(|err| format!("toolchain `{channel}`: {err:#}"))?;

    Ok(Toolchain {
        has_rustfmt: components.lines().any(|line| line.starts_with("rustfmt")),
        target_dir: target_dir(workspace),
        root,
        source: Source::Rustup(channel.to_owned()),
        rustc_version,
    })
}

fn no_toolchain_message(channel: &str, detail: &str) -> String {
    format!(
        "no Rust toolchain with rustc-dev was found ({detail})\n{}",
        install_hint(channel)
    )
}

fn rustup_output(args: &[&str]) -> Result<String> {
    let output = Command::new("rustup")
        .args(args)
        .output()
        .with_context(|| "running `rustup`")?;
    if !output.status.success() {
        bail!(
            "`rustup {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn toolchain_matches(line: &str, channel: &str) -> bool {
    let name = line.split_whitespace().next().unwrap_or_default();
    name == channel || name.starts_with(&format!("{channel}-"))
}

fn rustc_version(rustc: &Path) -> Result<String> {
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", rustc.display()))?;
    if !output.status.success() {
        bail!(
            "`{} --version` failed: {}",
            rustc.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.lines().next().unwrap_or_default().trim();
    if version.is_empty() {
        bail!("`{} --version` printed nothing", rustc.display());
    }
    Ok(version.to_owned())
}

fn rustc_dev_present(root: &Path) -> bool {
    #[cfg(windows)]
    let (dir, prefix, suffix) = (root.join("bin"), "rustc_driver", ".dll");
    #[cfg(not(windows))]
    let (dir, prefix, suffix) = (root.join("lib"), "librustc_driver", "");
    has_entry_with(&dir, prefix, suffix)
}

fn rustc_dev_detail(root: &Path) -> String {
    #[cfg(windows)]
    let (pattern, dir) = ("rustc_driver*.dll", root.join("bin"));
    #[cfg(not(windows))]
    let (pattern, dir) = ("librustc_driver*", root.join("lib"));
    format!("no {pattern} in {}", dir.display())
}

fn has_entry_with(dir: &Path, prefix: &str, suffix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with(prefix) && name.ends_with(suffix)
    })
}

fn rustfmt_present(root: &Path) -> bool {
    root.join("bin").join(exe("rustfmt")).is_file()
}

fn target_dir(workspace: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                workspace.join(dir)
            }
        }
        None => workspace.join("target"),
    }
}

/// The environment operations shared by `std::process::Command` and `xshell::Cmd`.
pub trait CommandEnv: Sized {
    fn env_var(self, key: &str, value: impl AsRef<OsStr>) -> Self;
    fn remove_env_var(self, key: &str) -> Self;
}

impl CommandEnv for &mut Command {
    fn env_var(self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env(key, value);
        self
    }

    fn remove_env_var(self, key: &str) -> Self {
        self.env_remove(key);
        self
    }
}

impl<'a> CommandEnv for Cmd<'a> {
    fn env_var(self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env(key, value)
    }

    fn remove_env_var(self, key: &str) -> Self {
        self.env_remove(key)
    }
}

impl Toolchain {
    #[allow(dead_code)] // used by the autozng test modes
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[allow(dead_code)] // used by the autozng test modes
    pub fn rustc_version(&self) -> &str {
        &self.rustc_version
    }

    pub fn has_rustfmt(&self) -> bool {
        self.has_rustfmt
    }

    pub fn cargo(&self) -> PathBuf {
        self.root.join("bin").join(exe("cargo"))
    }

    pub fn rustc(&self) -> PathBuf {
        self.root.join("bin").join(exe("rustc"))
    }

    /// A one-line description for logs.
    pub fn describe(&self) -> String {
        let source = match &self.source {
            Source::Env => SYSROOT_ENV.to_owned(),
            Source::Rustup(channel) => format!("rustup {channel}"),
        };
        format!("{} ({source}, {})", self.root.display(), self.rustc_version)
    }

    /// Build the driver and frontend in the nested workspace.
    #[allow(dead_code)] // used by the autozng test modes
    pub fn build_driver(&self, sh: &Shell, workspace: &Path) -> Result<DriverBinaries> {
        let cargo = self.cargo();
        {
            let _dir = sh.push_dir(workspace);
            self.apply_env(cmd!(sh, "{cargo} build -p autozng-driver -p cargo-autozng"))
                .run()?;
        }
        let bin = self.target_dir.join("debug");
        Ok(DriverBinaries {
            driver: bin.join(exe("autozng-driver")),
            frontend: bin.join(exe("cargo-autozng")),
        })
    }

    /// A command for the driver with the environment it needs to start.
    #[allow(dead_code)] // used by the autozng test modes
    pub fn driver_command(&self, driver: &Path) -> Command {
        let mut command = Command::new(driver);
        command.env("RUSTC_TOOLCHAIN", &self.root);

        let library_dir = if cfg!(windows) {
            self.root.join("bin")
        } else {
            self.root.join("lib")
        };
        command.env("PATH", prepend_to_path(&library_dir, "PATH"));
        #[cfg(unix)]
        {
            command.env(
                "LD_LIBRARY_PATH",
                prepend_to_path(&library_dir, "LD_LIBRARY_PATH"),
            );
            command.env(
                "DYLD_FALLBACK_LIBRARY_PATH",
                prepend_to_path(&library_dir, "DYLD_FALLBACK_LIBRARY_PATH"),
            );
        }
        command
    }

    /// Apply the toolchain environment to a command.
    #[allow(dead_code)] // used by the autozng test modes
    pub fn apply_env<C: CommandEnv>(&self, cmd: C) -> C {
        let mut cmd = cmd
            .env_var("PATH", self.path_with_bin())
            .env_var("CARGO", self.cargo())
            .env_var("RUSTC", self.rustc())
            .env_var("CARGO_TARGET_DIR", &self.target_dir);
        let rustdoc = self.root.join("bin").join(exe("rustdoc"));
        if rustdoc.is_file() {
            cmd = cmd.env_var("RUSTDOC", rustdoc);
        }
        for var in SCRUBBED_ENV {
            cmd = cmd.remove_env_var(var);
        }
        cmd
    }

    fn path_with_bin(&self) -> OsString {
        prepend_to_path(&self.root.join("bin"), "PATH")
    }
}

fn prepend_to_path(dir: &Path, env_var: &str) -> OsString {
    let mut entries = vec![dir.to_path_buf()];
    entries.extend(std::env::split_paths(
        &std::env::var_os(env_var).unwrap_or_default(),
    ));
    std::env::join_paths(entries).unwrap_or_else(|_| dir.as_os_str().to_owned())
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::{parse_channel, toolchain_matches};

    #[test]
    fn parses_the_channel_pin() {
        let text = "[toolchain]\n# pin the nightly\nchannel = \"nightly-2026-08-29\"\n";
        assert_eq!(parse_channel(text).as_deref(), Some("nightly-2026-08-29"));
        assert_eq!(parse_channel("[toolchain]\n"), None);
    }

    #[test]
    fn matches_installed_toolchain_names() {
        let line = "nightly-2026-08-29-x86_64-pc-windows-msvc (active)";
        assert!(toolchain_matches(line, "nightly-2026-08-29"));
        assert!(!toolchain_matches(line, "nightly-2026-08-28"));
        assert!(!toolchain_matches(line, "nightly-2026-08-2"));
        assert!(toolchain_matches("custom-nightly", "custom-nightly"));
    }
}
