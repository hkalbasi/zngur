use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Error, Result, bail};
use xshell::{Cmd, Shell, cmd};

use crate::{autozng, repo_root};

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

/// The tool-registration prelude the autozng driver needs.
const AUTOZNG_PRELUDE: &str = "\
#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
";

fn first_nonempty(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty())
}

/// The `//@` directive of a declarative test.
fn directive(zng_file: &Path) -> Result<&'static str> {
    let text = std::fs::read_to_string(zng_file)
        .with_context(|| format!("reading {}", zng_file.display()))?;
    let Some(line) = first_nonempty(&text) else {
        bail!("missing a test directive in {}", zng_file.display());
    };
    let Some(directive) = line.strip_prefix("//@") else {
        bail!("missing a test directive in {}", zng_file.display());
    };
    match directive.trim() {
        "pass" => Ok("pass"),
        "fail" => Ok("fail"),
        "compile_fail" => Ok("compile_fail"),
        other => bail!("unknown directive `//@{other}` in {}", zng_file.display()),
    }
}

/// The mode selected by a `//@` directive on the first non-empty line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Autozng,
    AutozngFail,
    CargoAutozng,
}

fn test_directive(path: &Path) -> Result<Option<String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(first_nonempty(&text)
        .and_then(|line| line.strip_prefix("//@"))
        .map(|directive| directive.trim().to_owned()))
}

fn rs_mode(path: &Path) -> Result<Mode> {
    let Some(directive) = test_directive(path)? else {
        bail!("missing a test directive in {}", path.display());
    };
    match directive.as_str() {
        "autozng" => Ok(Mode::Autozng),
        "autozng_fail" => Ok(Mode::AutozngFail),
        "cargo_autozng" => Ok(Mode::CargoAutozng),
        other => bail!("unknown directive `//@{other}` in {}", path.display()),
    }
}

#[derive(Default)]
struct Annotations {
    error_code: Option<String>,
    error_contains: Option<String>,
    contains: Vec<String>,
    no_stderr: bool,
}

fn read_annotations(path: &Path) -> Result<Annotations> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_annotations(&text, path)
}

fn parse_annotations(text: &str, path: &Path) -> Result<Annotations> {
    let mut annotations = Annotations::default();
    for line in text.lines() {
        let Some(body) = line.trim().strip_prefix("//@") else {
            continue;
        };
        let body = body.trim();
        if body == "no_stderr" {
            annotations.no_stderr = true;
            continue;
        }
        let Some((key, value)) = body.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "error_code" => {
                if annotations.error_code.replace(value.to_owned()).is_some() {
                    bail!(
                        "{} has more than one `error_code` annotation",
                        path.display()
                    );
                }
            }
            "error_contains" => {
                if annotations
                    .error_contains
                    .replace(value.to_owned())
                    .is_some()
                {
                    bail!(
                        "{} has more than one `error_contains` annotation",
                        path.display()
                    );
                }
            }
            "contains" => annotations.contains.push(value.to_owned()),
            _ => {}
        }
    }
    Ok(annotations)
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

/// Create the scratch crate that builds a generated bridge. Shared by
/// `//@ pass`, `//@ compile_fail`, and the autozng bridge checks.
fn prepare_runner(runner: &Path) -> Result<()> {
    std::fs::create_dir_all(runner.join("src"))?;
    std::fs::write(runner.join("Cargo.toml"), RUNNER_CARGO_TOML)?;
    std::fs::write(runner.join("build.rs"), RUNNER_BUILD_RS)?;
    // The CLI only writes the C++ file when it has glue to emit, so a test whose
    // `.zng` exports nothing would otherwise build the previous test's file.
    let generated_cpp = runner.join("generated.cpp");
    if generated_cpp.exists() {
        std::fs::remove_file(&generated_cpp)
            .with_context(|| format!("removing {}", generated_cpp.display()))?;
    }
    Ok(())
}

/// Run `zngur-cli generate` over `zng_file` into `runner`.
fn generate_bridge(sh: &Shell, zng_file: &Path, runner: &Path) -> Result<std::process::Output> {
    for file in [
        runner.join("src/generated.rs"),
        runner.join("generated.h"),
        runner.join("generated.cpp"),
    ] {
        if file.exists() {
            std::fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
        }
    }

    let input = Path::new(
        zng_file
            .file_name()
            .with_context(|| format!("{} has no file name", zng_file.display()))?,
    );
    let rs_file = runner.join("src/generated.rs");
    let h_file = runner.join("generated.h");
    let cpp_file = runner.join("generated.cpp");
    let _dir = sh.push_dir(zng_file.parent().context("a .zng file has a parent")?);
    Ok(cmd!(
        sh,
        "cargo run -q -p zngur-cli -- generate {input} -i --crate-name crate --rs-file {rs_file} --h-file {h_file} --cpp-file {cpp_file}"
    )
    .ignore_status()
    .output()?)
}

#[derive(Clone, Copy)]
enum RunnerTarget {
    Bin,
    Lib,
}

/// Build the bridge runner, denying warnings when requested.
fn build_runner(
    sh: &Shell,
    runner: &Path,
    cpp: &Path,
    target: RunnerTarget,
    deny_warnings: bool,
) -> Result<std::process::Output> {
    let build = match (target, deny_warnings) {
        (RunnerTarget::Bin, true) => {
            cmd!(sh, "cargo rustc -q --bin zngur-e2e-runner -- -D warnings")
        }
        (RunnerTarget::Bin, false) => {
            cmd!(sh, "cargo rustc -q --bin zngur-e2e-runner -- -A warnings")
        }
        (RunnerTarget::Lib, true) => cmd!(sh, "cargo rustc -q --lib -- -D warnings"),
        (RunnerTarget::Lib, false) => cmd!(sh, "cargo rustc -q --lib -- -A warnings"),
    };
    let _dir = sh.push_dir(runner);
    Ok(build
        .env("ZNGUR_E2E_CPP", cpp)
        .env("CARGO_TARGET_DIR", runner.join("target"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .ignore_status()
        .output()?)
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
    prepare_runner(&runner)?;

    let zng_file = test_dir.join(format!("{stem}.zng"));
    let generate = generate_bridge(sh, &zng_file, &runner)?;
    if !generate.status.success() {
        bail!(
            "`{stem}.zng` is marked `//@ pass` but generating it failed:\n{}",
            String::from_utf8_lossy(&generate.stderr)
        );
    }
    check_stderr(fix, stem, test_dir, &normalize(&generate.stderr), false)?;

    std::fs::copy(&rs_file, runner.join("src/main.rs"))
        .with_context(|| format!("copying {}", rs_file.display()))?;

    // Build first so that compiler output doesn't end up in the snapshot, then
    // run the binary with stdout and stderr redirected to the same file so the
    // interleaving matches what a user sees on a terminal.
    let build = build_runner(sh, &runner, &cpp_file, RunnerTarget::Bin, true)?;
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

    let annotations = read_annotations(&zng_file)?;
    let expected_code = match annotations.error_code {
        Some(code) if !code.is_empty() => Some(code),
        Some(_) => bail!("`{stem}.zng` has an empty `error_code` annotation"),
        None => None,
    };
    let expected_message = annotations
        .error_contains
        .filter(|value| !value.is_empty())
        .with_context(|| {
            format!("`{stem}.zng` needs an `//@ error_contains: MESSAGE` annotation")
        })?;

    let runner = repo_root().join("target/e2e-compile-fail");
    prepare_runner(&runner)?;

    let generate = generate_bridge(sh, &zng_file, &runner)?;
    if !generate.status.success() {
        bail!(
            "`{stem}.zng` is marked `//@ compile_fail` but generating it failed:\n{}",
            String::from_utf8_lossy(&generate.stderr)
        );
    }

    std::fs::copy(&rs_file, runner.join("src/main.rs"))
        .with_context(|| format!("copying {}", rs_file.display()))?;

    let build = build_runner(sh, &runner, &cpp_file, RunnerTarget::Bin, true)?;
    if build.status.success() {
        bail!("`{stem}.zng` was expected to fail compilation but compiled successfully");
    }

    let stderr = String::from_utf8_lossy(&build.stderr);
    if let Some(code) = expected_code.as_deref()
        && !stderr.contains(&format!("error[{code}]"))
    {
        bail!(
            "`{stem}.zng` failed to compile, but compiler output did not contain error code `{code}`:\n{stderr}"
        );
    }
    if !stderr.contains(&expected_message) {
        bail!(
            "`{stem}.zng` failed to compile, but compiler output did not contain `{expected_message}`:\n{stderr}"
        );
    }
    for diagnostic in stderr.lines().filter(|line| line.starts_with("error")) {
        if diagnostic.starts_with("error: could not compile")
            || diagnostic.starts_with("error: build failed")
            || diagnostic.starts_with("error: failed to run custom build command")
            || diagnostic.starts_with("error: aborting due to")
        {
            continue;
        }
        let expected_diagnostic = expected_code
            .as_deref()
            .is_some_and(|code| diagnostic.starts_with(&format!("error[{code}]")))
            || (diagnostic.starts_with("error:") && diagnostic.contains(&expected_message));
        if !expected_diagnostic {
            bail!(
                "`{stem}.zng` failed with an unexpected compiler diagnostic `{diagnostic}`:\n{stderr}"
            );
        }
    }

    Ok(())
}

fn strip_autozng_directives(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//@"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn autozng_scratch(root: &Path, tests_dir: &Path, source: &Path) -> PathBuf {
    let relative = source.strip_prefix(tests_dir).unwrap_or(source);
    root.join("target/autozng-tests")
        .join(relative.with_extension(""))
}

fn prepare_autozng_source(source: &Path, scratch: &Path) -> Result<()> {
    let text =
        std::fs::read_to_string(source).with_context(|| format!("reading {}", source.display()))?;
    let body = strip_autozng_directives(&text);
    let lib = scratch.join("src/lib.rs");
    std::fs::create_dir_all(
        lib.parent()
            .with_context(|| format!("{} has no parent", lib.display()))?,
    )?;
    std::fs::write(&lib, format!("{AUTOZNG_PRELUDE}{body}"))
        .with_context(|| format!("writing {}", lib.display()))?;
    Ok(())
}

fn run_autozng_driver(
    toolchain: &autozng::Toolchain,
    driver: &Path,
    scratch: &Path,
) -> Result<std::process::Output> {
    let mut command = toolchain.driver_command(driver);
    command
        .current_dir(scratch)
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("--crate-name=scratch_crate")
        .arg("--output-zng")
        .arg("out.zng")
        .arg("src/lib.rs")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
        .output()
        .with_context(|| format!("running {}", driver.display()))
}

fn normalize_driver_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(stderr)).replace("\r\n", "\n")
}

fn push_diagnostic_block(blocks: &mut Vec<String>, current: &mut Vec<&str>) {
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

fn is_diagnostic_header(line: &str) -> bool {
    line.starts_with("warning: ")
        || line.starts_with("error: ")
        || line.starts_with("warning[")
        || line.starts_with("error[")
}

fn is_autozng_diagnostic(block: &str) -> bool {
    block.starts_with("warning: autozng:") || block.starts_with("error: autozng:")
}

fn autozng_diagnostics(stderr: &[u8]) -> String {
    let text = normalize_driver_stderr(stderr);

    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if is_diagnostic_header(line) {
            push_diagnostic_block(&mut blocks, &mut current);
        }
        current.push(line);
    }
    push_diagnostic_block(&mut blocks, &mut current);

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

fn check_autozng_stderr(fix: bool, path: &Path, actual: &str, name: &str) -> Result<()> {
    if fix {
        if actual.is_empty() {
            if path.exists() {
                std::fs::remove_file(path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        } else {
            std::fs::write(path, format!("{actual}\n"))
                .with_context(|| format!("writing {}", path.display()))?;
        }
        return Ok(());
    }
    check_snapshot(fix, path, actual, false, name)
}

fn reset_runner_source(runner: &Path) -> Result<()> {
    let src = runner.join("src");
    if src.exists() {
        std::fs::remove_dir_all(&src).with_context(|| format!("removing {}", src.display()))?;
    }
    std::fs::create_dir_all(&src).with_context(|| format!("creating {}", src.display()))?;
    Ok(())
}

fn defer_bridge_failure(err: Error) -> String {
    let text = format!("{err:#}");
    let first = text.lines().next().unwrap_or("bridge check failed");
    format!("bridge check deferred: {first}")
}

fn check_autozng_bridge(sh: &Shell, scratch: &Path, name: &str) -> Result<()> {
    let runner = repo_root().join("target/autozng-bridge");
    prepare_runner(&runner)?;
    reset_runner_source(&runner)?;

    let source = std::fs::read_to_string(scratch.join("src/lib.rs"))
        .with_context(|| format!("reading {}", scratch.join("src/lib.rs").display()))?;
    std::fs::write(
        runner.join("src/main.rs"),
        format!("{source}\nmod generated;\nfn main() {{}}\n"),
    )
    .with_context(|| format!("writing {}", runner.join("src/main.rs").display()))?;

    let cpp = runner.join("impls.cpp");
    std::fs::write(&cpp, "// no hand-written C++ for autozng cases\n")
        .with_context(|| format!("writing {}", cpp.display()))?;

    let zng_file = scratch.join("out.zng");
    let generate = generate_bridge(sh, &zng_file, &runner)?;
    if !generate.status.success() {
        bail!(
            "generated bridge for `{name}` failed to generate:\n{}",
            normalize_output(&generate.stderr)
        );
    }

    let build = build_runner(sh, &runner, &cpp, RunnerTarget::Bin, false)?;
    if !build.status.success() {
        bail!(
            "generated bridge for `{name}` failed to build:\n{}",
            normalize_output(&build.stderr)
        );
    }
    Ok(())
}

fn check_autozng(
    sh: &Shell,
    fix: bool,
    source: &Path,
    tests_dir: &Path,
    toolchain: &autozng::Toolchain,
    binaries: &autozng::DriverBinaries,
    allow_autozng_bridge_failures: bool,
) -> Result<Option<String>> {
    let root = repo_root();
    let name = source
        .strip_prefix(&root)
        .unwrap_or(source)
        .display()
        .to_string();
    let annotations = read_annotations(source)?;
    let scratch = autozng_scratch(&root, tests_dir, source);
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("removing {}", scratch.display()))?;
    }
    prepare_autozng_source(source, &scratch)?;

    let output = run_autozng_driver(toolchain, &binaries.driver, &scratch)?;
    if !output.status.success() {
        bail!(
            "`{name}` driver failed ({}):\n{}",
            output.status,
            normalize_driver_stderr(&output.stderr)
        );
    }

    let out_zng = scratch.join("out.zng");
    let zng = std::fs::read_to_string(&out_zng)
        .with_context(|| format!("reading {}", out_zng.display()))?
        .replace("\r\n", "\n");
    let zng = zng.trim_end().to_owned();
    check_snapshot(fix, &source.with_extension("zng"), &zng, true, &name)?;

    if annotations.no_stderr {
        let path = source.with_extension("stderr");
        if fix && path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    } else {
        let diagnostics = autozng_diagnostics(&output.stderr);
        check_autozng_stderr(
            fix,
            &source.with_extension("stderr"),
            diagnostics.trim_end(),
            &name,
        )?;
    }

    if zng.is_empty() {
        return Ok(None);
    }

    match check_autozng_bridge(sh, &scratch, &name) {
        Ok(()) => Ok(None),
        Err(err) if allow_autozng_bridge_failures => Ok(Some(defer_bridge_failure(err))),
        Err(err) => Err(err),
    }
}

fn check_autozng_fail(
    source: &Path,
    tests_dir: &Path,
    toolchain: &autozng::Toolchain,
    binaries: &autozng::DriverBinaries,
) -> Result<()> {
    let root = repo_root();
    let name = source
        .strip_prefix(&root)
        .unwrap_or(source)
        .display()
        .to_string();
    let annotations = read_annotations(source)?;
    let expected_code = match annotations.error_code {
        Some(code) if !code.is_empty() => Some(code),
        Some(_) => bail!("`{name}` has an empty `error_code` annotation"),
        None => None,
    };
    let expected_message = annotations
        .error_contains
        .filter(|value| !value.is_empty())
        .with_context(|| format!("`{name}` needs an `//@ error_contains: MESSAGE` annotation"))?;

    let scratch = autozng_scratch(&root, tests_dir, source);
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("removing {}", scratch.display()))?;
    }
    prepare_autozng_source(source, &scratch)?;

    let output = run_autozng_driver(toolchain, &binaries.driver, &scratch)?;
    if output.status.success() {
        bail!("`{name}` was expected to fail but the driver accepted it");
    }

    let stderr = normalize_driver_stderr(&output.stderr);
    if let Some(code) = expected_code.as_deref()
        && !stderr.contains(&format!("error[{code}]"))
    {
        bail!(
            "`{name}` failed, but the driver output did not contain error code `{code}`:\n{stderr}"
        );
    }
    if !stderr.contains(&expected_message) {
        bail!(
            "`{name}` failed, but the driver output did not contain `{expected_message}`:\n{stderr}"
        );
    }
    for diagnostic in stderr.lines().filter(|line| line.starts_with("error")) {
        if diagnostic.starts_with("error: could not compile")
            || diagnostic.starts_with("error: build failed")
            || diagnostic.starts_with("error: aborting due to")
        {
            continue;
        }
        let expected = expected_code
            .as_deref()
            .is_some_and(|code| diagnostic.starts_with(&format!("error[{code}]")))
            || (diagnostic.starts_with("error:") && diagnostic.contains(&expected_message));
        if !expected {
            bail!("`{name}` failed with an unexpected diagnostic `{diagnostic}`:\n{stderr}");
        }
    }
    Ok(())
}

fn package_name(manifest: &Path) -> Result<String> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("name") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches(['"', '\'']);
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
    }
    bail!("no `package.name` in {}", manifest.display())
}

fn run_cargo_autozng(
    sh: &Shell,
    example_dir: &Path,
    toolchain: &autozng::Toolchain,
    binaries: &autozng::DriverBinaries,
    allow_autozng_bridge_failures: bool,
) -> Result<Option<String>> {
    let root = repo_root();
    let name = example_dir
        .strip_prefix(&root)
        .unwrap_or(example_dir)
        .display()
        .to_string();
    let source = ["src/main.rs", "src/lib.rs"]
        .into_iter()
        .map(|file| example_dir.join(file))
        .find(|file| file.is_file())
        .with_context(|| format!("`{name}` has no src/main.rs or src/lib.rs"))?;
    if rs_mode(&source)? != Mode::CargoAutozng {
        bail!("`{name}` is missing `//@ cargo_autozng`");
    }
    let annotations = read_annotations(&source)?;
    let crate_name = package_name(&example_dir.join("Cargo.toml"))?;
    let scratch = root
        .join("target/autozng-tests/e2e")
        .join(example_dir.file_name().context("example has no name")?);
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("removing {}", scratch.display()))?;
    }
    let zng_dir = scratch.join("zng");
    let target_dir = scratch.join("target");
    std::fs::create_dir_all(&zng_dir)?;

    let manifest = example_dir.join("Cargo.toml");
    let mut command = std::process::Command::new(&binaries.frontend);
    command
        .current_dir(&root)
        .arg("--driver")
        .arg(&binaries.driver)
        .arg("--output-dir")
        .arg(&zng_dir)
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--manifest-path")
        .arg(&manifest);
    toolchain.apply_env(&mut command);
    let output = command
        .output()
        .with_context(|| format!("running {}", binaries.frontend.display()))?;
    if !output.status.success() {
        bail!(
            "`{name}` cargo-autozng failed ({}):\n--- stdout ---\n{}--- stderr ---\n{}",
            output.status,
            normalize_output(&output.stdout),
            normalize_output(&output.stderr),
        );
    }

    let zng_path = zng_dir.join(format!("{crate_name}.zng"));
    let zng = std::fs::read_to_string(&zng_path)
        .with_context(|| format!("reading {}", zng_path.display()))?;
    for marker in &annotations.contains {
        if !zng.contains(marker) {
            bail!("`{name}` generated `.zng` is missing `{marker}`");
        }
    }

    let runner = root.join("target/autozng-bridge");
    prepare_runner(&runner)?;
    reset_runner_source(&runner)?;
    copy_dir_all(&example_dir.join("src"), &runner.join("src"))?;
    let target = if example_dir.join("src/main.rs").is_file() {
        RunnerTarget::Bin
    } else {
        RunnerTarget::Lib
    };
    let crate_root = match target {
        RunnerTarget::Bin => runner.join("src/main.rs"),
        RunnerTarget::Lib => runner.join("src/lib.rs"),
    };
    let text = std::fs::read_to_string(&crate_root)
        .with_context(|| format!("reading {}", crate_root.display()))?;
    std::fs::write(&crate_root, format!("{text}\nmod generated;\n"))
        .with_context(|| format!("writing {}", crate_root.display()))?;

    let cpp = runner.join("impls.cpp");
    std::fs::write(&cpp, "// no hand-written C++ for autozng examples\n")
        .with_context(|| format!("writing {}", cpp.display()))?;

    let bridge = || -> Result<()> {
        let generate = generate_bridge(sh, &zng_path, &runner)?;
        if !generate.status.success() {
            bail!(
                "generated bridge for `{name}` failed to generate:\n{}",
                normalize_output(&generate.stderr)
            );
        }
        let build = build_runner(sh, &runner, &cpp, target, false)?;
        if !build.status.success() {
            bail!(
                "generated bridge for `{name}` failed to build:\n{}",
                normalize_output(&build.stderr)
            );
        }
        Ok(())
    };
    match bridge() {
        Ok(()) => Ok(None),
        Err(err) if allow_autozng_bridge_failures => Ok(Some(defer_bridge_failure(err))),
        Err(err) => Err(err),
    }
}

fn run_cargo_suite(
    sh: &Shell,
    workspace: &Path,
    toolchain: &autozng::Toolchain,
    binaries: &autozng::DriverBinaries,
    fix: bool,
) -> Result<()> {
    if fix {
        eprintln!("note: `--fix` does not update the zngur-autozng cargo-test suite");
        return Ok(());
    }
    let cargo = toolchain.cargo();
    let _dir = sh.push_dir(workspace);
    run_example_command(
        toolchain
            .apply_env(cmd!(sh, "{cargo} test -p autozng-driver -p cargo-autozng"))
            .env("AUTOZNG_DRIVER", &binaries.driver),
        "zngur-autozng cargo-test suite",
    )
}

fn ensure_binaries<'a>(
    sh: &Shell,
    toolchain: &autozng::Toolchain,
    workspace: &Path,
    cache: &'a mut Option<autozng::DriverBinaries>,
) -> Result<&'a autozng::DriverBinaries> {
    if cache.is_none() {
        *cache = Some(toolchain.build_driver(sh, workspace)?);
    }
    Ok(cache.as_ref().expect("just built"))
}

/// A declarative test, free-form Nushell test, build-and-run example, or
/// autozng case.
enum TestEntry {
    Zng(PathBuf),
    NuDir(PathBuf),
    Example(PathBuf),
    /// A `tests/autozng/**/*.rs` fixture.
    Autozng(PathBuf),
    /// A `zngur-autozng/examples/*` crate with `//@ cargo_autozng`.
    CargoAutozng(PathBuf),
    /// The `zngur-autozng` cargo-test suite.
    CargoSuite(PathBuf),
}

const CARGO_EXAMPLES: &[&str] = &["cxx_demo", "tutorial_cpp", "unsafe_fns"];

impl TestEntry {
    fn path(&self) -> &Path {
        match self {
            TestEntry::Zng(path)
            | TestEntry::NuDir(path)
            | TestEntry::Example(path)
            | TestEntry::Autozng(path)
            | TestEntry::CargoAutozng(path)
            | TestEntry::CargoSuite(path) => path,
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
    Ok(first_nonempty(&text).is_some_and(|line| line.starts_with("//@")))
}

/// Collect `.rs` fixtures under `dir` whose first non-empty line is
/// `//@ autozng` or `//@ autozng_fail`.
fn collect_autozng_tests(
    root: &Path,
    dir: &Path,
    filters: &[String],
    out: &mut Vec<TestEntry>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            if path.join("main.nu").is_file() {
                continue;
            }
            collect_autozng_tests(root, &path, filters, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            match test_directive(&path)?.as_deref() {
                Some("autozng" | "autozng_fail") => {
                    if path_matches(root, &path, filters) {
                        out.push(TestEntry::Autozng(path));
                    }
                }
                Some("cargo_autozng") => bail!(
                    "`//@ cargo_autozng` is only valid in zngur-autozng/examples: {}",
                    path.display()
                ),
                Some(directive) => {
                    bail!("unknown directive `//@{directive}` in {}", path.display())
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Collect example crates whose crate root starts with `//@ cargo_autozng`.
fn collect_cargo_autozng_examples(
    root: &Path,
    filters: &[String],
    out: &mut Vec<TestEntry>,
) -> Result<()> {
    let examples = root.join("zngur-autozng/examples");
    let Ok(entries) = std::fs::read_dir(&examples) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let source = ["src/main.rs", "src/lib.rs"]
            .into_iter()
            .map(|file| path.join(file))
            .find(|file| file.is_file());
        let Some(source) = source else {
            continue;
        };
        if test_directive(&source)?.as_deref() != Some("cargo_autozng") {
            continue;
        }
        if path_matches(root, &path, filters) || path_matches(root, &source, filters) {
            out.push(TestEntry::CargoAutozng(path));
        }
    }
    Ok(())
}

/// Whether `path` matches any of the glob `filters`, which are matched against
/// the path relative to the repository root. An empty filter list matches
/// everything.
fn path_matches(root: &Path, path: &Path, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
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
    use super::{example_matches, glob_matches, path_matches};
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
    fn windows_separators_match() {
        let root = Path::new(r"C:\repo");
        let path = root.join(r"tests\parser\foo.zng");
        assert!(path_matches(root, &path, &["tests/parser/*".to_owned()]));
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

/// Run every test under `tests/`, recursively. Directories group tests by
/// feature area; a `*.zng` gets its mode from the directive in the file, while a
/// directory with a `main.nu` is a free-form Nushell test.
pub fn main_with(
    fix: bool,
    filter: Option<String>,
    filter_file: Option<PathBuf>,
    fail_list: Option<PathBuf>,
    verbose: bool,
    only_autozng: bool,
    require_autozng: bool,
    allow_autozng_bridge_failures: bool,
) -> Result<()> {
    let sh = &Shell::new()?;
    let root = repo_root();
    let tests_dir = root.join("tests");
    let workspace = autozng::workspace_root(&root);

    let mut filters = Vec::new();
    if let Some(filter) = filter {
        filters.push(filter);
    }
    if let Some(path) = filter_file {
        filters.extend(read_filter_file(&path)?);
    }

    let mut tests = Vec::new();
    if !only_autozng {
        collect_tests(&root, &tests_dir, &filters, &mut tests)?;
        collect_examples(&root, &filters, &mut tests)?;
    }

    let mut autozng_entries = Vec::new();
    collect_autozng_tests(&root, &tests_dir, &filters, &mut autozng_entries)?;
    collect_cargo_autozng_examples(&root, &filters, &mut autozng_entries)?;
    let autozng_in_scope = filters.is_empty()
        || only_autozng
        || require_autozng
        || !autozng_entries.is_empty()
        || path_matches(&root, &workspace, &filters);

    let mut autozng_skip = None;
    let toolchain = if autozng_in_scope {
        match autozng::resolve(&workspace) {
            autozng::Resolution::Found(toolchain) => {
                eprintln!("zngur-autozng toolchain: {}", toolchain.describe());
                Some(toolchain)
            }
            autozng::Resolution::Skipped(reason) => {
                if require_autozng {
                    bail!("{reason}");
                }
                eprintln!("warning: skipping zngur-autozng tests: {reason}");
                autozng_skip = Some(reason.lines().next().unwrap_or(&reason).to_owned());
                None
            }
        }
    } else {
        None
    };

    if autozng_in_scope {
        tests.extend(autozng_entries);
        if filters.is_empty() || only_autozng || path_matches(&root, &workspace, &filters) {
            tests.push(TestEntry::CargoSuite(workspace.clone()));
        }
    }

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
    let mut skipped_names: Vec<(String, String)> = Vec::new();
    let mut cli_built = false;
    let mut binaries: Option<autozng::DriverBinaries> = None;

    for test in &tests {
        let name = test
            .path()
            .strip_prefix(&root)
            .unwrap_or(test.path())
            .display()
            .to_string();

        let result: Result<Option<String>> = match test {
            TestEntry::NuDir(dir) => {
                if !cli_built {
                    cmd!(sh, "cargo build -q -p zngur-cli").quiet().run()?;
                    cli_built = true;
                }
                run_nu_dir(dir, &tests_dir, fix).map(|_| None)
            }
            TestEntry::Example(dir) => run_example(sh, dir, fix).map(|_| None),
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
                .map(|_| None)
            }
            TestEntry::Autozng(source) => {
                if let Some(reason) = &autozng_skip {
                    Ok(Some(reason.clone()))
                } else {
                    (|| {
                        let toolchain = toolchain
                            .as_ref()
                            .expect("autozng entries imply a toolchain");
                        let binaries = ensure_binaries(sh, toolchain, &workspace, &mut binaries)?;
                        match rs_mode(source)? {
                            Mode::Autozng => check_autozng(
                                sh,
                                fix,
                                source,
                                &tests_dir,
                                toolchain,
                                binaries,
                                allow_autozng_bridge_failures,
                            ),
                            Mode::AutozngFail => {
                                check_autozng_fail(source, &tests_dir, toolchain, binaries)
                                    .map(|_| None)
                            }
                            Mode::CargoAutozng => bail!(
                                "`//@ cargo_autozng` is only valid in zngur-autozng/examples: {}",
                                source.display()
                            ),
                        }
                    })()
                }
            }
            TestEntry::CargoAutozng(dir) => {
                if let Some(reason) = &autozng_skip {
                    Ok(Some(reason.clone()))
                } else {
                    (|| {
                        let toolchain = toolchain
                            .as_ref()
                            .expect("autozng entries imply a toolchain");
                        let binaries = ensure_binaries(sh, toolchain, &workspace, &mut binaries)?;
                        run_cargo_autozng(
                            sh,
                            dir,
                            toolchain,
                            binaries,
                            allow_autozng_bridge_failures,
                        )
                    })()
                }
            }
            TestEntry::CargoSuite(dir) => {
                if let Some(reason) = &autozng_skip {
                    Ok(Some(reason.clone()))
                } else {
                    (|| {
                        let toolchain = toolchain
                            .as_ref()
                            .expect("autozng entries imply a toolchain");
                        let binaries = ensure_binaries(sh, toolchain, &workspace, &mut binaries)?;
                        run_cargo_suite(sh, dir, toolchain, binaries, fix).map(|_| None)
                    })()
                }
            }
        };

        match result {
            Ok(None) => {
                passed += 1;
                if verbose {
                    eprintln!("ok {name}");
                } else {
                    eprint!(".");
                }
            }
            Ok(Some(reason)) => {
                skipped_names.push((name.clone(), reason));
                if verbose {
                    eprintln!("skip {name}");
                } else {
                    eprint!("s");
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
    for (name, reason) in &skipped_names {
        eprintln!("SKIP {name}: {reason}");
    }
    eprintln!(
        "\nsummary: passed={}, failed={}, skipped={}  ({:.2}s)",
        passed,
        failed_names.len(),
        skipped_names.len(),
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
