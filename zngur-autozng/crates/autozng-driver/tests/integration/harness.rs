//! Scratch directories and the driver runtime environment for the plumbing tests.
//!
//! The handshake and wrapper tests run the real `autozng-driver` binary as a subprocess. The
//! driver links the toolchain's rustc libraries, so these helpers provide its loader path and a
//! scratch crate.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The toolchain the driver was built against.
///
/// The driver is dynamically linked against the toolchain's rustc libraries; it needs their
/// directory on `PATH` and the toolchain root in `RUSTC_TOOLCHAIN` (see `AGENTS.md`). The
/// sysroot is discovered from `rustc`, which resolves the pinned toolchain via
/// `rust-toolchain.toml`; `AUTOZNG_TEST_SYSROOT` overrides it for unusual setups.
pub(crate) fn toolchain() -> &'static Path {
    static TOOLCHAIN: OnceLock<PathBuf> = OnceLock::new();
    TOOLCHAIN
        .get_or_init(|| {
            if let Some(dir) = env::var_os("AUTOZNG_TEST_SYSROOT") {
                return PathBuf::from(dir);
            }
            let output = Command::new("rustc")
                .args(["--print", "sysroot"])
                .output()
                .expect(
                    "failed to run `rustc --print sysroot` to locate the toolchain; \
                 set AUTOZNG_TEST_SYSROOT to the toolchain root instead",
                );
            assert!(
                output.status.success(),
                "`rustc --print sysroot` failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        })
        .as_path()
}

/// The real `rustc` of the toolchain, as cargo passes it to a wrapper as `argv[1]`.
pub(crate) fn rustc_path() -> PathBuf {
    toolchain()
        .join("bin")
        .join(format!("rustc{}", env::consts::EXE_SUFFIX))
}

/// A [`Command`] for the driver binary with the environment it needs to start already set.
///
/// The driver links against the toolchain's rustc libraries. On Windows those DLLs live in
/// `bin` (found through `PATH`); on Unix the shared objects live in `lib` (found through the
/// loader path). Prepend the right one so the driver can start, then let the OS search the
/// existing path as before. Callers set their own arguments and current directory.
pub(crate) fn driver_command() -> Command {
    let toolchain = toolchain();
    let mut command = Command::new(env!("CARGO_BIN_EXE_autozng-driver"));
    command.env("RUSTC_TOOLCHAIN", toolchain);

    let library_dir = if cfg!(windows) {
        toolchain.join("bin")
    } else {
        toolchain.join("lib")
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

/// `env_var`'s value with `dir` prepended to the platform's path list.
fn prepend_to_path(dir: &Path, env_var: &str) -> std::ffi::OsString {
    let mut entries = vec![dir.to_path_buf()];
    entries.extend(env::split_paths(&env::var_os(env_var).unwrap_or_default()));
    env::join_paths(entries).expect("invalid path entries")
}

pub(crate) fn write_files(root: &Path, files: &[(&str, &str)]) {
    assert!(!files.is_empty(), "a scratch crate needs at least one file");
    for (rel, contents) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|err| panic!("failed to create `{}`: {err}", parent.display()));
        }
        fs::write(&path, contents)
            .unwrap_or_else(|err| panic!("failed to write `{}`: {err}", path.display()));
    }
}

/// A scratch crate directory, deleted when dropped unless `AUTOZNG_TEST_KEEP` is set or the
/// test is already unwinding (so a failing test leaves its crate behind for inspection).
pub(crate) struct Scratch {
    pub(crate) path: PathBuf,
}

impl Scratch {
    pub(crate) fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .replace("::", "-");
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        // The pid keeps concurrent `cargo test` invocations from fighting over the same name.
        let path = env::temp_dir()
            .join("autozng-integration")
            .join(format!("{thread}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap_or_else(|err| {
            panic!("failed to create scratch dir `{}`: {err}", path.display())
        });

        Scratch { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let keep = env::var_os("AUTOZNG_TEST_KEEP").is_some() || std::thread::panicking();
        if !keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
