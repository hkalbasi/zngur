use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use xshell::{Cmd, Shell, cmd};

/// A throwaway cargo project used to build a `//@ pass` test. The test's `.rs`
/// is copied to `src/main.rs` and its C++ file is compiled into the binary by
/// `build.rs`. Kept out of the workspace so no manifest is needed in `tests/`.
const RUNNER_CARGO_TOML: &str = r#"[package]
name = "zngur-e2e-runner"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
zngur-lib = { path = "../../zngur-lib" }

[build-dependencies]
cc = "1.0"
"#;

const RUNNER_BUILD_RS: &str = r#"use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let cpp = PathBuf::from(std::env::var("ZNGUR_E2E_CPP").unwrap());

    let mut build = cc::Build::new();
    build.cpp(true).std("c++20").include(&dir);
    // The test's own headers live next to its `.cpp`, so a `#cpp_additional_includes`
    // can pull them in even though it is emitted into the runner's `generated.h`.
    let test_dir = cpp.parent().unwrap();
    build.include(test_dir);
    // Strip the test directory from `__FILE__` so snapshots of `zngur_dbg` output
    // don't depend on where the repository is checked out.
    let prefix = format!("{}{}", test_dir.display(), std::path::MAIN_SEPARATOR);
    if build.get_compiler().is_like_msvc() && !build.get_compiler().is_like_clang() {
        build.flag("/Zc:__cplusplus");
        build.flag(format!("/d1trimfile:{prefix}"));
    } else {
        build.flag(format!("-ffile-prefix-map={prefix}="));
    }
    let generated_cpp = dir.join("generated.cpp");
    if generated_cpp.exists() {
        build.file(generated_cpp);
    }
    build.file(&cpp).compile("zngur_e2e");

    println!("cargo:rerun-if-changed=generated.cpp");
    println!("cargo:rerun-if-changed={}", cpp.display());
    println!("cargo:rerun-if-env-changed=ZNGUR_E2E_CPP");
}
"#;

/// Repository root, resolved from this crate's manifest so that the paths inside
/// the diagnostics don't depend on the directory `cargo xtask` was invoked from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the repository")
        .to_owned()
}

/// The `//@` directive of a declarative test.
fn directive(zng_file: &Path) -> Result<&'static str> {
    let text = std::fs::read_to_string(zng_file)
        .with_context(|| format!("reading {}", zng_file.display()))?;
    for line in text.lines() {
        if let Some(directive) = line.trim().strip_prefix("//@") {
            return match directive.trim() {
                "pass" => Ok("pass"),
                "fail" => Ok("fail"),
                "compile_fail" => Ok("compile_fail"),
                other => bail!("unknown directive `//@{other}` in {}", zng_file.display()),
            };
        }
    }
    bail!("missing a test directive in {}", zng_file.display())
}

/// Strip colors from the CLI output and drop the trailing panic report, which
/// contains a non-deterministic thread id.
fn normalize(stderr: &[u8]) -> String {
    let stderr = strip_ansi_escapes::strip(stderr);
    String::from_utf8_lossy(&stderr)
        .lines()
        .take_while(|line| !line.starts_with("thread 'main'"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// Normalize the output of a test binary: strip colors and use `\n` line
/// endings so snapshots are stable across platforms.
fn normalize_output(output: &[u8]) -> String {
    let output = strip_ansi_escapes::strip(output);
    String::from_utf8_lossy(&output)
        .lines()
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// `zngur generate` must reject `<stem>.zng`. The diagnostics it prints,
/// including the source spans, are compared with the `<stem>.stderr` snapshot.
fn check_fail(sh: &Shell, fix: bool, stem: &str, test_dir: &Path) -> Result<()> {
    let out_dir = repo_root().join("target/e2e-fail");
    // Creating the output directory first makes an unexpectedly accepted file
    // fail the check below instead of failing to write its generated files.
    std::fs::create_dir_all(&out_dir)?;
    let rs_file = out_dir.join("generated.rs");
    let h_file = out_dir.join("generated.h");
    let cpp_file = out_dir.join("generated.cpp");

    // The CLI runs in the test's directory, so spans point at `<stem>.zng`.
    let output = {
        let _dir = sh.push_dir(test_dir);
        cmd!(
            sh,
            "cargo run -q -p zngur-cli -- generate {stem}.zng --rs-file {rs_file} --h-file {h_file} --cpp-file {cpp_file}"
        )
        .ignore_status()
        .output()?
    };
    if output.status.success() {
        bail!("`{stem}.zng` is marked `//@ fail` but the CLI accepted it");
    }

    check_stderr(fix, stem, test_dir, &normalize(&output.stderr), true)
}

/// Compare `actual` with the snapshot at `snapshot_file`, or write it when `fix`
/// is set. When `required` is false a missing snapshot is allowed only if there
/// is no output; output without a snapshot is an error.
fn check_snapshot(
    fix: bool,
    snapshot_file: &Path,
    actual: &str,
    required: bool,
    name: &str,
) -> Result<()> {
    if fix {
        // Don't create an empty snapshot for a passing test that has none.
        if required || !actual.is_empty() || snapshot_file.exists() {
            std::fs::write(snapshot_file, format!("{actual}\n"))
                .with_context(|| format!("writing {}", snapshot_file.display()))?;
        }
        return Ok(());
    }
    let expected = match std::fs::read_to_string(snapshot_file) {
        Ok(expected) => expected,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
            if actual.is_empty() {
                return Ok(());
            }
            bail!(
                "`{name}` produced output but has no `{}` snapshot. Run `cargo xtask test --fix` to create it.\n--- actual ---\n{actual}",
                snapshot_file.display()
            );
        }
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", snapshot_file.display()));
        }
    };
    if expected.trim_end() != actual {
        bail!(
            "`{name}` output differs from `{}`. Run `cargo xtask test --fix` to update the snapshot.\n--- expected ---\n{expected}\n--- actual ---\n{actual}",
            snapshot_file.display()
        );
    }
    Ok(())
}

/// Compare `actual` (normalized stderr) with the `<stem>.stderr` snapshot.
fn check_stderr(
    fix: bool,
    stem: &str,
    test_dir: &Path,
    actual: &str,
    required: bool,
) -> Result<()> {
    check_snapshot(
        fix,
        &test_dir.join(format!("{stem}.stderr")),
        actual,
        required,
        &format!("{stem}.zng"),
    )
}

/// Generate the bridge for `<stem>.zng`, build `<stem>.rs` with `<stem>.cpp`
/// linked into it, and run it. The test must exit successfully. Its combined
/// stdout and stderr are compared with `<stem>.output`. Generation warnings
/// (e.g. from deprecated items) are compared with `<stem>.stderr`; either
/// snapshot must exist whenever the corresponding step emits anything.
fn check_pass(sh: &Shell, fix: bool, stem: &str, test_dir: &Path) -> Result<()> {
    let rs_file = test_dir.join(format!("{stem}.rs"));
    let cpp_file = test_dir.join(format!("{stem}.cpp"));
    if !rs_file.is_file() {
        bail!("`{stem}.zng` is marked `//@ pass` but `{stem}.rs` is missing");
    }
    if !cpp_file.is_file() {
        bail!("`{stem}.zng` is marked `//@ pass` but `{stem}.cpp` is missing");
    }

    let runner = repo_root().join("target/e2e");
    std::fs::create_dir_all(runner.join("src"))?;
    std::fs::write(runner.join("Cargo.toml"), RUNNER_CARGO_TOML)?;
    std::fs::write(runner.join("build.rs"), RUNNER_BUILD_RS)?;

    {
        let _dir = sh.push_dir(test_dir);
        let generate = cmd!(
            sh,
            "cargo run -q -p zngur-cli -- generate {stem}.zng -i --crate-name crate --rs-file {runner}/src/generated.rs --h-file {runner}/generated.h --cpp-file {runner}/generated.cpp"
        )
        .ignore_status()
        .output()?;
        if !generate.status.success() {
            bail!(
                "`{stem}.zng` is marked `//@ pass` but generating it failed:\n{}",
                String::from_utf8_lossy(&generate.stderr)
            );
        }
        check_stderr(fix, stem, test_dir, &normalize(&generate.stderr), false)?;
    }

    std::fs::copy(&rs_file, runner.join("src/main.rs"))
        .with_context(|| format!("copying {}", rs_file.display()))?;

    // Build first so that compiler output doesn't end up in the snapshot, then
    // run the binary with stdout and stderr redirected to the same file so the
    // interleaving matches what a user sees on a terminal.
    let build = {
        let _dir = sh.push_dir(&runner);
        cmd!(sh, "cargo rustc -q --bin zngur-e2e-runner -- -D warnings")
            .env("ZNGUR_E2E_CPP", &cpp_file)
            .env("CARGO_TARGET_DIR", runner.join("target"))
            .ignore_status()
            .output()?
    };
    if !build.status.success() {
        bail!(
            "`{stem}.zng` is marked `//@ pass` but it failed to build:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
    }

    let exe = runner
        .join("target/debug")
        .join(format!("zngur-e2e-runner{}", std::env::consts::EXE_SUFFIX));
    let log_file = runner.join("output.log");
    let log = std::fs::File::create(&log_file)
        .with_context(|| format!("creating {}", log_file.display()))?;
    let status = std::process::Command::new(&exe)
        .current_dir(&runner)
        .stdout(std::process::Stdio::from(log.try_clone()?))
        .stderr(std::process::Stdio::from(log))
        .status()
        .with_context(|| format!("running {}", exe.display()))?;
    let output = normalize_output(&std::fs::read(&log_file)?);
    if !status.success() {
        bail!("`{stem}.zng` is marked `//@ pass` but it failed to run:\n{output}");
    }

    check_snapshot(
        fix,
        &test_dir.join(format!("{stem}.output")),
        &output,
        false,
        &format!("{stem}.zng"),
    )
}

/// Build a test expected to fail in rustc, matching its error code and a stable
/// message substring instead of snapshotting compiler diagnostics.
fn check_compile_fail(sh: &Shell, stem: &str, test_dir: &Path) -> Result<()> {
    let rs_file = test_dir.join(format!("{stem}.rs"));
    let cpp_file = test_dir.join(format!("{stem}.cpp"));
    let zng_file = test_dir.join(format!("{stem}.zng"));
    if !rs_file.is_file() {
        bail!("`{stem}.zng` is marked `//@ compile_fail` but `{stem}.rs` is missing");
    }
    if !cpp_file.is_file() {
        bail!("`{stem}.zng` is marked `//@ compile_fail` but `{stem}.cpp` is missing");
    }

    let zng = std::fs::read_to_string(&zng_file)
        .with_context(|| format!("reading {}", zng_file.display()))?;
    let mut expected_code = None;
    let mut expected_message = None;
    for annotation in zng
        .lines()
        .filter_map(|line| line.trim().strip_prefix("//@"))
    {
        let Some((key, value)) = annotation.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "error_code" => {
                if expected_code.replace(value).is_some() {
                    bail!("`{stem}.zng` has more than one `error_code` annotation");
                }
            }
            "error_contains" => {
                if expected_message.replace(value).is_some() {
                    bail!("`{stem}.zng` has more than one `error_contains` annotation");
                }
            }
            _ => {}
        }
    }
    let expected_code = match expected_code {
        Some(code) if !code.is_empty() => Some(code),
        Some(_) => bail!("`{stem}.zng` has an empty `error_code` annotation"),
        None => None,
    };
    let expected_message = expected_message
        .filter(|value| !value.is_empty())
        .with_context(|| {
            format!("`{stem}.zng` needs an `//@ error_contains: MESSAGE` annotation")
        })?;

    let runner = repo_root().join("target/e2e-compile-fail");
    std::fs::create_dir_all(runner.join("src"))?;
    std::fs::write(runner.join("Cargo.toml"), RUNNER_CARGO_TOML)?;
    std::fs::write(runner.join("build.rs"), RUNNER_BUILD_RS)?;
    let generated_cpp = runner.join("generated.cpp");
    if generated_cpp.exists() {
        std::fs::remove_file(&generated_cpp)
            .with_context(|| format!("removing {}", generated_cpp.display()))?;
    }

    {
        let _dir = sh.push_dir(test_dir);
        let generate = cmd!(
            sh,
            "cargo run -q -p zngur-cli -- generate {stem}.zng -i --crate-name crate --rs-file {runner}/src/generated.rs --h-file {runner}/generated.h --cpp-file {runner}/generated.cpp"
        )
        .ignore_status()
        .output()?;
        if !generate.status.success() {
            bail!(
                "`{stem}.zng` is marked `//@ compile_fail` but generating it failed:\n{}",
                String::from_utf8_lossy(&generate.stderr)
            );
        }
    }

    std::fs::copy(&rs_file, runner.join("src/main.rs"))
        .with_context(|| format!("copying {}", rs_file.display()))?;

    let build = {
        let _dir = sh.push_dir(&runner);
        cmd!(sh, "cargo rustc -q --bin zngur-e2e-runner -- -D warnings")
            .env("ZNGUR_E2E_CPP", &cpp_file)
            .env("CARGO_TARGET_DIR", runner.join("target"))
            .ignore_status()
            .output()?
    };
    if build.status.success() {
        bail!("`{stem}.zng` was expected to fail compilation but compiled successfully");
    }

    let stderr = String::from_utf8_lossy(&build.stderr);
    if let Some(expected_code) = expected_code
        && !stderr.contains(&format!("error[{expected_code}]"))
    {
        bail!(
            "`{stem}.zng` failed to compile, but compiler output did not contain error code `{expected_code}`:\n{stderr}"
        );
    }
    if !stderr.contains(expected_message) {
        bail!(
            "`{stem}.zng` failed to compile, but compiler output did not contain `{expected_message}`:\n{stderr}"
        );
    }
    for diagnostic in stderr.lines().filter(|line| line.starts_with("error")) {
        if diagnostic.starts_with("error: could not compile")
            || diagnostic.starts_with("error: build failed")
        {
            continue;
        }
        let expected_diagnostic = expected_code
            .is_some_and(|code| diagnostic.starts_with(&format!("error[{code}]")))
            || (diagnostic.starts_with("error:") && diagnostic.contains(expected_message));
        if !expected_diagnostic {
            bail!(
                "`{stem}.zng` failed with an unexpected compiler diagnostic `{diagnostic}`:\n{stderr}"
            );
        }
    }

    Ok(())
}

/// A declarative test, free-form Nushell test, or build-and-run example.
enum TestEntry {
    Zng(PathBuf),
    NuDir(PathBuf),
    Example(PathBuf),
}

const CARGO_EXAMPLES: &[&str] = &["cxx_demo", "tutorial_cpp", "unsafe_fns"];

impl TestEntry {
    fn path(&self) -> &Path {
        match self {
            TestEntry::Zng(path) | TestEntry::NuDir(path) | TestEntry::Example(path) => path,
        }
    }
}

fn collect_examples(root: &Path, filters: &[String], out: &mut Vec<TestEntry>) -> Result<()> {
    let examples_dir = root.join("examples");
    for entry in std::fs::read_dir(&examples_dir)
        .with_context(|| format!("reading {}", examples_dir.display()))?
    {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("example directory has a non-UTF-8 name")?;
        let runnable = if CARGO_EXAMPLES.contains(&name) {
            true
        } else if cfg!(windows) {
            path.join("NMakefile").is_file()
        } else {
            path.join("Makefile").is_file()
        };
        if !runnable || !example_matches(root, &path, filters) {
            continue;
        }
        out.push(TestEntry::Example(path));
    }
    Ok(())
}

fn example_matches(root: &Path, path: &Path, filters: &[String]) -> bool {
    if filters.is_empty() || path_matches(root, path, filters) {
        return true;
    }

    [
        "main.zng",
        "main.cpp",
        "Cargo.toml",
        "Makefile",
        "NMakefile",
    ]
    .into_iter()
    .map(|file| path.join(file))
    .chain(["src/main.rs"].into_iter().map(|file| path.join(file)))
    .any(|file| path_matches(root, &file, filters))
}

/// Collect tests under `dir`, recursively. A directory containing `main.nu` is a
/// free-form Nushell test and is not searched further. A `*.zng` file without a
/// `//@` directive is a fixture imported by another test (via `merge`), not a
/// test of its own.
fn collect_tests(
    root: &Path,
    dir: &Path,
    filters: &[String],
    out: &mut Vec<TestEntry>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            if path.join("main.nu").is_file() {
                if path_matches(root, &path, filters) {
                    out.push(TestEntry::NuDir(path));
                }
            } else {
                collect_tests(root, &path, filters, out)?;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("zng")
            && has_directive(&path)?
            && path_matches(root, &path, filters)
        {
            out.push(TestEntry::Zng(path));
        }
    }
    Ok(())
}

/// Whether a `*.zng` file starts with a `//@` test directive and is therefore a
/// test rather than a fixture.
fn has_directive(zng_file: &Path) -> Result<bool> {
    let text = std::fs::read_to_string(zng_file)
        .with_context(|| format!("reading {}", zng_file.display()))?;
    Ok(text
        .lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with("//@")))
}

/// Whether `path` matches any of the glob `filters`, which are matched against
/// the path relative to the repository root. An empty filter list matches
/// everything.
fn path_matches(root: &Path, path: &Path, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    filters
        .iter()
        .any(|pattern| glob_matches(pattern, &relative))
}

/// Minimal glob matching: `*` stays within a path segment, `**` crosses
/// segments, and `?` matches one non-separator character.
fn glob_matches(pattern: &str, text: &str) -> bool {
    fn inner(pattern: &[u8], text: &[u8]) -> bool {
        if pattern.is_empty() {
            return text.is_empty();
        }
        match pattern[0] {
            b'*' if pattern.get(1) == Some(&b'*') => {
                inner(&pattern[2..], text) || (!text.is_empty() && inner(pattern, &text[1..]))
            }
            b'*' => {
                inner(&pattern[1..], text)
                    || (!text.is_empty() && text[0] != b'/' && inner(pattern, &text[1..]))
            }
            b'?' => !text.is_empty() && text[0] != b'/' && inner(&pattern[1..], &text[1..]),
            b => !text.is_empty() && b == text[0] && inner(&pattern[1..], &text[1..]),
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{example_matches, glob_matches};
    use std::path::Path;

    #[test]
    fn glob_matching() {
        assert!(glob_matches("tests/parser/*", "tests/parser/foo.zng"));
        assert!(!glob_matches(
            "tests/parser/*",
            "tests/parser/nested/foo.zng"
        ));
        assert!(glob_matches(
            "tests/**/*.zng",
            "tests/parser/nested/foo.zng"
        ));
        assert!(glob_matches("tests/**", "tests/cli/foo/main.nu"));
    }

    #[test]
    fn example_filter_matches_source_path() {
        let root = Path::new("/repo");
        let example = root.join("examples/simple");
        assert!(example_matches(
            root,
            &example,
            &["examples/simple/*".to_owned()]
        ));
    }
}

fn run_example_command(command: Cmd<'_>, action: &str) -> Result<()> {
    let output = command
        .ignore_status()
        .output()
        .with_context(|| format!("{action} command could not be started"))?;
    if !output.status.success() {
        bail!(
            "{action} failed with {}\n--- stdout ---\n{}--- stderr ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

fn run_example(sh: &Shell, dir: &Path, fix: bool) -> Result<()> {
    let example = dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("example directory has a non-UTF-8 name")?;
    let _dir = sh.push_dir(dir);
    let profile = std::env::var("EXAMPLES_CARGO_PROFILE").unwrap_or_else(|_| "debug".to_owned());
    let (profile, target_dir) = if profile == "debug" {
        ("dev", "debug")
    } else {
        (profile.as_str(), profile.as_str())
    };

    if CARGO_EXAMPLES.contains(&example) {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = cmd!(
                sh,
                "rm -f generated.h generated.cpp src/generated.rs actual_output.txt"
            )
            .ignore_status()
            .output();
            run_example_command(
                cmd!(sh, "cargo build --profile {profile}").env("RUSTFLAGS", "-D warnings"),
                &format!("building example `{example}`"),
            )?;

            let bash_cmd = format!(
                "../../target/{target_dir}/example-{example} 2>&1 | sed -e s/thread.*\\(.*\\)/thread/g > actual_output.txt"
            );
            run_example_command(
                cmd!(sh, "bash -c {bash_cmd}"),
                &format!("running example `{example}`"),
            )?;
        }
        #[cfg(target_os = "windows")]
        {
            let _ = cmd!(
                sh,
                "cmd /c 'del /f /q generated.h generated.cpp src\\generated.rs actual_output.txt 2>nul'"
            )
            .ignore_status()
            .output();
            let build_cmd = format!("cargo build --profile {profile}");
            run_example_command(
                cmd!(sh, "cmd /c {build_cmd}").env("RUSTFLAGS", "-D warnings"),
                &format!("building example `{example}`"),
            )?;
            let run_cmd =
                format!("..\\..\\target\\{target_dir}\\example-{example} > actual_output.txt 2>&1");
            run_example_command(
                cmd!(sh, "cmd /c {run_cmd}"),
                &format!("running example `{example}`"),
            )?;
            run_example_command(
                cmd!(
                    sh,
                    "pwsh -Command '(Get-Content actual_output.txt) -replace \"thread.*\\(.*\\)\", \"thread\" -replace \"\\\\\", \"/\"| Out-File actual_output.txt'"
                ),
                &format!("filtering example `{example}` output"),
            )?;
        }
    } else {
        #[cfg(not(target_os = "windows"))]
        {
            run_example_command(
                cmd!(sh, "make clean"),
                &format!("cleaning example `{example}`"),
            )?;
            run_example_command(
                cmd!(sh, "make").env("RUSTFLAGS", "-D warnings"),
                &format!("building example `{example}`"),
            )?;
            run_example_command(
                cmd!(
                    sh,
                    "bash -c './a.out 2>&1 | sed -e s/thread.*\\(.*\\)/thread/g > actual_output.txt'"
                ),
                &format!("running example `{example}`"),
            )?;
            if sh.path_exists("b.out") {
                run_example_command(
                    cmd!(
                        sh,
                        "bash -c './b.out 2>&1 | sed -r s/thread.*\\(.*\\)/thread/g >> actual_output.txt'"
                    ),
                    &format!("running example `{example}` b.out"),
                )?;
            }
        }
        #[cfg(target_os = "windows")]
        {
            run_example_command(
                cmd!(sh, "nmake /f NMakefile clean"),
                &format!("cleaning example `{example}`"),
            )?;
            run_example_command(
                cmd!(sh, "nmake /f NMakefile").env("RUSTFLAGS", "-D warnings"),
                &format!("building example `{example}`"),
            )?;
            if sh.path_exists("a.bat") {
                run_example_command(
                    cmd!(sh, "cmd /c '.\\a.bat > actual_output.txt 2>&1'"),
                    &format!("running example `{example}`"),
                )?;
            } else {
                run_example_command(
                    cmd!(sh, "cmd /c '.\\a.exe > actual_output.txt 2>&1'"),
                    &format!("running example `{example}`"),
                )?;
            }
            if sh.path_exists("b.exe") {
                run_example_command(
                    cmd!(sh, "cmd /c '.\\b.exe >> actual_output.txt 2>&1'"),
                    &format!("running example `{example}` b.exe"),
                )?;
            }
            run_example_command(
                cmd!(
                    sh,
                    "pwsh -Command '(Get-Content actual_output.txt) -replace \"thread.*\\(.*\\)\", \"thread\" -replace \"\\\\\", \"/\"| Out-File actual_output.txt'"
                ),
                &format!("filtering example `{example}` output"),
            )?;
        }
    }

    if fix {
        sh.copy_file("actual_output.txt", "expected_output.txt")?;
    } else {
        #[cfg(not(target_os = "windows"))]
        run_example_command(
            cmd!(sh, "diff actual_output.txt expected_output.txt"),
            &format!("example `{example}` output differs from expected"),
        )?;
        #[cfg(target_os = "windows")]
        run_example_command(
            cmd!(sh, "cmd /c 'fc actual_output.txt expected_output.txt'"),
            &format!("example `{example}` output differs from expected"),
        )?;
    }

    run_example_command(
        cmd!(sh, "cargo fmt --check"),
        &format!("example `{example}` is not formatted. Run `cargo fmt`"),
    )?;
    Ok(())
}

/// Read glob filters from `path`, one per line. Blank lines and `#` comments
/// are ignored.
fn read_filter_file(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading filter file {}", path.display()))?;
    let filters: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    eprintln!("loaded {} filter(s) from {}", filters.len(), path.display());
    Ok(filters)
}

/// Run every test under `tests/` without filtering. Used by `cargo xtask ci`.
pub fn main(fix: bool) -> Result<()> {
    main_with(fix, None, None, None, false)
}

/// Run every test under `tests/`, recursively. Directories group tests by
/// feature area; a `*.zng` gets its mode from the directive in the file, while a
/// directory with a `main.nu` is a free-form Nushell test.
pub fn main_with(
    fix: bool,
    filter: Option<String>,
    filter_file: Option<PathBuf>,
    fail_list: Option<PathBuf>,
    verbose: bool,
) -> Result<()> {
    let sh = &Shell::new()?;
    let root = repo_root();
    let tests_dir = root.join("tests");

    let mut filters = Vec::new();
    if let Some(filter) = filter {
        filters.push(filter);
    }
    if let Some(path) = filter_file {
        filters.extend(read_filter_file(&path)?);
    }

    let mut tests = Vec::new();
    collect_tests(&root, &tests_dir, &filters, &mut tests)?;
    collect_examples(&root, &filters, &mut tests)?;
    tests.sort_by(|a, b| a.path().cmp(b.path()));
    if tests.is_empty() {
        if filters.is_empty() {
            bail!("no tests or examples found");
        }
        bail!("no tests matched the given filter(s)");
    }
    eprintln!("running {} tests", tests.len());

    let start = Instant::now();
    let mut passed = 0;
    let mut failed_names = Vec::new();
    let mut cli_built = false;

    for test in &tests {
        let name = test
            .path()
            .strip_prefix(&root)
            .unwrap_or(test.path())
            .display()
            .to_string();

        let result = match test {
            TestEntry::NuDir(dir) => {
                if !cli_built {
                    cmd!(sh, "cargo build -q -p zngur-cli").quiet().run()?;
                    cli_built = true;
                }
                run_nu_dir(dir, &tests_dir, fix)
            }
            TestEntry::Example(dir) => run_example(sh, dir, fix),
            TestEntry::Zng(zng_file) => {
                let stem = zng_file
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let test_dir = zng_file.parent().expect("a file has a parent directory");
                match directive(zng_file)? {
                    "fail" => check_fail(sh, fix, &stem, test_dir),
                    "compile_fail" => check_compile_fail(sh, &stem, test_dir),
                    _ => check_pass(sh, fix, &stem, test_dir),
                }
            }
        };

        match result {
            Ok(()) => {
                passed += 1;
                if verbose {
                    eprintln!("ok {name}");
                } else {
                    eprint!(".");
                }
            }
            Err(err) => {
                eprintln!("\nFAIL {name}\n{err:#}");
                failed_names.push(name);
            }
        }
    }

    let elapsed = start.elapsed();
    eprintln!();
    for name in &failed_names {
        eprintln!("FAIL {name}");
    }
    eprintln!(
        "\nsummary: passed={}, failed={}  ({:.2}s)",
        passed,
        failed_names.len(),
        elapsed.as_secs_f64(),
    );

    let fail_list = fail_list.unwrap_or_else(|| root.join("fail_list.txt"));
    if failed_names.is_empty() {
        if fail_list.exists() {
            std::fs::remove_file(&fail_list)
                .with_context(|| format!("removing {}", fail_list.display()))?;
        }
    } else {
        let mut content = failed_names.join("\n");
        content.push('\n');
        std::fs::write(&fail_list, content)
            .with_context(|| format!("writing {}", fail_list.display()))?;
        eprintln!("wrote failing test list to {}", fail_list.display());
    }

    if !failed_names.is_empty() {
        bail!("{} test(s) failed", failed_names.len());
    }
    Ok(())
}

/// Path of the `zngur` CLI built next to the running `xtask` (same cargo profile).
fn zngur_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolving the xtask executable path")?;
    let dir = exe
        .parent()
        .context("the xtask executable has no parent directory")?;
    Ok(dir.join(format!("zngur{}", std::env::consts::EXE_SUFFIX)))
}

/// The `#@ run-status` (or `//@ run-status`) directive of a Nushell test.
fn nu_run_status(main_nu: &Path) -> Result<i32> {
    let text = std::fs::read_to_string(main_nu)
        .with_context(|| format!("reading {}", main_nu.display()))?;
    for line in text.lines() {
        let line = line.trim_start();
        let Some(body) = line.strip_prefix("#@").or_else(|| line.strip_prefix("//@")) else {
            continue;
        };
        if let Some((key, value)) = body.trim().split_once(':')
            && key.trim() == "run-status"
        {
            return value.trim().parse().with_context(|| {
                format!("`run-status` must be an integer in {}", main_nu.display())
            });
        }
    }
    Ok(0)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Run a free-form Nushell test. The directory's `main.nu` is executed (via
/// `nu`) inside a scratch copy of the directory, so it may create files freely;
/// its exit code must match the `#@ run-status` directive (default 0).
/// `tests/snapshot-utils.nu` is copied alongside it for snapshot assertions.
fn run_nu_dir(test_dir: &Path, tests_dir: &Path, fix: bool) -> Result<()> {
    let name = test_dir.strip_prefix(tests_dir).unwrap_or(test_dir);
    let scratch = repo_root().join("target/nu-tests").join(name);
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("cleaning {}", scratch.display()))?;
    }
    copy_dir_all(test_dir, &scratch)?;

    let snapshot_utils = tests_dir.join("snapshot-utils.nu");
    if snapshot_utils.is_file() {
        std::fs::copy(&snapshot_utils, scratch.join("snapshot-utils.nu"))
            .context("copying snapshot-utils.nu into the test scratch directory")?;
    }

    let main_nu = scratch.join("main.nu");
    let expected_status = nu_run_status(&main_nu)?;

    let bin = zngur_bin()?;
    let bin_dir = bin
        .parent()
        .context("the zngur binary has no parent directory")?;

    let mut cmd = std::process::Command::new("nu");
    cmd.arg(&main_nu)
        .current_dir(&scratch)
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .env("ZNGUR_BIN", &bin)
        .env("ZNGUR_ROOT", repo_root())
        .env("ZNGUR_TEST_DIR", &scratch)
        .env("ZNGUR_TEST_SOURCE_DIR", test_dir);

    let mut paths = vec![bin_dir.to_owned()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("PATH", joined);
    }

    if fix {
        cmd.env("ZNGUR_UPDATE_SNAPSHOTS", "1");
    }

    let output = cmd
        .output()
        .with_context(|| format!("running Nushell test {}", main_nu.display()))?;
    let got_status = output.status.code().unwrap_or(-1);
    if got_status != expected_status {
        bail!(
            "Nushell test `{}` exit code mismatch: expected {expected_status}, got {got_status}\n--- stdout ---\n{}--- stderr ---\n{}",
            name.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}
