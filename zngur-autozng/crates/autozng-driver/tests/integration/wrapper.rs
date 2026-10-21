//! Wrapper-mode plumbing: cargo invokes the driver as `RUSTC_WORKSPACE_WRAPPER`, so the real
//! rustc arrives as `argv[1]`, the output location travels in the environment, and cargo's
//! information probes (`-vV`, `--print=…`) must be answered by the compiler unchanged.
//!
//! These tests are behavioural: they run the real driver binary exactly as cargo would. See
//! [`crate::harness`] for the toolchain environment and the scratch-directory rules.

use std::fs;
use std::process::{Command, Stdio};

use crate::harness;

/// A [`Command`] running the driver in wrapper mode, i.e. with the real rustc as `argv[1]`.
fn wrapped(driver_args: &[&str]) -> Command {
    let mut command = harness::driver_command();
    command.arg(harness::rustc_path());
    command.args(driver_args);
    command
}

#[test]
fn wrapper_mode_strips_the_rustc_path_and_honours_output_dir() {
    let scratch = harness::Scratch::new();
    harness::write_files(
        &scratch.path,
        &[("src/lib.rs", "pub struct Example { pub value: u32 }\n")],
    );
    let out = scratch.path.join("zng");
    fs::create_dir_all(&out).unwrap();

    let output = wrapped(&[
        "--edition=2024",
        "--crate-type=lib",
        "--crate-name=scratch_crate",
        "src/lib.rs",
    ])
    .current_dir(&scratch.path)
    .env("AUTOZNG_OUTPUT_DIR", &out)
    .stdin(Stdio::null())
    .output()
    .expect("failed to spawn the driver");

    assert!(
        output.status.success(),
        "the driver failed in wrapper mode (was the rustc path stripped?):\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let zng = fs::read_to_string(out.join("scratch_crate.zng"))
        .expect("no `.zng` was written into `AUTOZNG_OUTPUT_DIR`");
    assert!(
        zng.contains("type crate::Example"),
        "unexpected `.zng`:\n{zng}"
    );
}

#[test]
fn a_command_line_output_zng_wins_over_the_environment() {
    let scratch = harness::Scratch::new();
    harness::write_files(&scratch.path, &[("src/lib.rs", "pub struct Example;\n")]);

    let output = wrapped(&[
        "--edition=2024",
        "--crate-type=lib",
        "--crate-name=scratch_crate",
        "--output-zng",
        "cli.zng",
        "src/lib.rs",
    ])
    .current_dir(&scratch.path)
    .env("AUTOZNG_OUTPUT_ZNG", "env.zng")
    .stdin(Stdio::null())
    .output()
    .expect("failed to spawn the driver");

    assert!(
        output.status.success(),
        "the driver failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        scratch.path.join("cli.zng").exists(),
        "`--output-zng` was ignored"
    );
    assert!(
        !scratch.path.join("env.zng").exists(),
        "`AUTOZNG_OUTPUT_ZNG` should have been overridden by `--output-zng`"
    );
}

#[test]
fn information_queries_are_answered_by_the_compiler_unchanged() {
    let scratch = harness::Scratch::new();

    let output = wrapped(&["--print=cfg"])
        .current_dir(&scratch.path)
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn the driver");
    assert!(
        output.status.success(),
        "the `--print=cfg` probe failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let probed = String::from_utf8_lossy(&output.stdout);
    assert!(
        !probed.contains("autozng"),
        "the `--print=cfg` probe leaked the injected `autozng` cfg:\n{probed}"
    );

    // And it is not merely "without the extra cfg" — it is the compiler's own answer.
    let reference = Command::new(harness::rustc_path())
        .arg("--print=cfg")
        .output()
        .expect("failed to run the real rustc");
    assert_eq!(
        probed,
        String::from_utf8_lossy(&reference.stdout),
        "the wrapped `--print=cfg` differs from the real rustc's"
    );
}
