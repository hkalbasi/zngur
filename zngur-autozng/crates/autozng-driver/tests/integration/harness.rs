//! Harness that runs the real `autozng-driver` executable against a throwaway crate.
//!
//! A test hands the harness the source of a small crate (as a string, or as a set of files),
//! the harness writes it into a fresh scratch directory under the system temp dir, invokes the
//! driver binary on it and hands back the rendered `.zng` plus the captured process output.
//! The scratch directory is removed when the returned [`Run`] is dropped; set
//! `AUTOZNG_TEST_KEEP=1` (or let the test fail) to keep it around for debugging.
//!
//! The driver is run exactly the way a user would run it: as a subprocess that receives rustc
//! arguments. The only thing the harness adds is the environment the driver needs to find the
//! toolchain it was built against (see [`toolchain`]).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use expect_test::Expect;

/// The tool-registration prelude that [`run`] prepends to every source.
///
/// `autozng` is a registered tool, so tool attributes only exist once it is registered; this
/// also enables the nightly features that registration needs. Tests that build their crate
/// file-by-file (via [`run_files`]) control this themselves.
pub const PRELUDE: &str = "\
#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
";

/// Run the driver over a single-file crate whose body is `body`.
///
/// The crate is written to `src/lib.rs` and [`PRELUDE`] is prepended automatically, so a test
/// can focus on the items under test.
pub fn run(body: &str) -> Run {
    let root = format!("{PRELUDE}{body}");
    run_files(&[("src/lib.rs", &root)])
}

/// Run the driver over a crate assembled from `files` (paths relative to the scratch root).
///
/// ``src/lib.rs`` is the crate root. The contents are written verbatim, so callers that want
/// tool attributes must include [`PRELUDE`] themselves.
pub fn run_files(files: &[(&str, &str)]) -> Run {
    let scratch = Scratch::new();
    write_files(&scratch.path, files);
    let output = invoke_driver(&scratch.path);
    let zng = fs::read_to_string(scratch.path.join("out.zng"))
        .ok()
        .map(normalize_newlines);

    Run {
        dir: scratch,
        status: output.status,
        stdout: normalize_newlines(String::from_utf8_lossy(&output.stdout).into_owned()),
        stderr: normalize_newlines(String::from_utf8_lossy(&output.stderr).into_owned()),
        zng,
    }
}

/// Rewrite Windows line endings (`\r\n`) to `\n` so snapshots do not depend on the host.
///
/// The driver and `rustc` emit `\r\n` on Windows (the `.zng` writer already uses `\n`), which
/// would otherwise leak into the captured output and make the same test fail on another OS.
fn normalize_newlines(text: String) -> String {
    text.replace("\r\n", "\n")
}

/// The outcome of one driver invocation.
pub struct Run {
    /// Keeps the scratch directory alive for as long as the result is; it is deleted on drop.
    dir: Scratch,
    /// Exit status of the driver process.
    pub status: ExitStatus,
    /// Captured stdout of the driver process, with line endings normalized to `\n`.
    pub stdout: String,
    /// Captured stderr of the driver process, with line endings normalized to `\n`.
    pub stderr: String,
    /// Contents of the generated `.zng` (line endings normalized to `\n`), or `None` if the
    /// driver did not write it.
    zng: Option<String>,
}

impl Run {
    /// Snapshot the rendered `.zng` against `expected`.
    ///
    /// `expect![[...]].assert_eq(...)` at the call site would force rustfmt to break the chain
    /// onto its own line; taking the [`Expect`] as an argument keeps the literal readable and
    /// matches how the parent `zngur-parser` tests are written.
    pub fn assert_zng(&self, expected: Expect) {
        expected.assert_eq(self.zng());
    }

    /// Snapshot the [`Run::autozng_diagnostics`] against `expected`.
    pub fn assert_diagnostics(&self, expected: Expect) {
        expected.assert_eq(&self.autozng_diagnostics());
    }

    /// The rendered `.zng`.
    ///
    /// Panics with the captured process output when the driver failed (or wrote no file), so a
    /// broken test points at the compiler diagnostics rather than at a confusing empty diff.
    pub fn zng(&self) -> &str {
        self.zng.as_deref().unwrap_or_else(|| {
            panic!(
                "the driver did not produce `out.zng` (status: {})\n\
                 scratch dir: {}\n\
                 re-run with `AUTOZNG_TEST_KEEP=1` to keep it\n\
                 --- stderr ---\n{}\n--- stdout ---\n{}",
                self.status,
                self.dir.path.display(),
                self.stderr,
                self.stdout,
            )
        })
    }

    /// The `autozng` diagnostics (warnings and errors) extracted from stderr.
    ///
    /// Plain rustc lints are dropped, as is the driver's own progress chatter, so the result is
    /// stable enough to snapshot. Path separators are normalized to `/` and the diagnostics
    /// refer to `src/lib.rs` (the source path the harness passes), never to an absolute path.
    ///
    /// The non-empty result keeps a trailing newline, matching both rustc's own output and the
    /// rendered `.zng`, so `expect![[...]]` literals (which always end in a newline) compare
    /// equal without a fiddly adjustment.
    pub fn autozng_diagnostics(&self) -> String {
        let text = strip_ansi(&self.stderr);

        let mut blocks: Vec<String> = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        for line in text.lines() {
            if is_diagnostic_header(line) {
                push_block(&mut blocks, &mut current);
            }
            current.push(line);
        }
        push_block(&mut blocks, &mut current);

        let joined = blocks
            .into_iter()
            .filter(|block| is_autozng_diagnostic(block))
            .map(|block| block.replace('\\', "/"))
            .collect::<Vec<_>>()
            .join("\n\n");
        if joined.is_empty() {
            joined
        } else {
            joined + "\n"
        }
    }
}

/// Split out a diagnostic block, trimming the blank separator lines rustc leaves behind.
fn push_block(blocks: &mut Vec<String>, current: &mut Vec<&str>) {
    if current.is_empty() {
        return;
    }
    let block = current.join("\n");
    let block = block.trim().to_string();
    current.clear();
    if !block.is_empty() {
        blocks.push(block);
    }
}

/// Whether a line starts a rustc diagnostic (as opposed to indented continuation output).
fn is_diagnostic_header(line: &str) -> bool {
    line.starts_with("warning: ")
        || line.starts_with("error: ")
        || line.starts_with("warning[")
        || line.starts_with("error[")
}

fn is_autozng_diagnostic(block: &str) -> bool {
    block.starts_with("warning: autozng:") || block.starts_with("error: autozng:")
}

/// The toolchain the driver was built against.
///
/// The driver is dynamically linked against the toolchain's rustc libraries; it needs their
/// directory on `PATH` and the toolchain root in `RUSTC_TOOLCHAIN` (see `AGENTS.md`). The
/// sysroot is discovered from `rustc`, which resolves the pinned toolchain via
/// `rust-toolchain.toml`; `AUTOZNG_TEST_SYSROOT` overrides it for unusual setups.
fn toolchain() -> &'static Path {
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

fn invoke_driver(dir: &Path) -> std::process::Output {
    let toolchain = toolchain();

    let mut command = Command::new(env!("CARGO_BIN_EXE_autozng-driver"));
    command
        .current_dir(dir)
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("--crate-name=scratch_crate")
        .arg("--output-zng")
        .arg("out.zng")
        .arg("src/lib.rs")
        .env("RUSTC_TOOLCHAIN", toolchain)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // The driver links against the toolchain's rustc libraries. On Windows those DLLs live in
    // `bin` (found through `PATH`); on Unix the shared objects live in `lib` (found through the
    // loader path). Prepend the right one so the driver can start, then let the OS search the
    // existing path as before.
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
        .output()
        .expect("failed to spawn the autozng driver")
}

/// `env_var`'s value with `dir` prepended to the platform's path list.
fn prepend_to_path(dir: &Path, env_var: &str) -> std::ffi::OsString {
    let mut entries = vec![dir.to_path_buf()];
    entries.extend(env::split_paths(&env::var_os(env_var).unwrap_or_default()));
    env::join_paths(entries).expect("invalid path entries")
}

fn write_files(root: &Path, files: &[(&str, &str)]) {
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
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
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

/// Remove ANSI SGR/CSI escape sequences rustc may add when it thinks stderr is a terminal.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
