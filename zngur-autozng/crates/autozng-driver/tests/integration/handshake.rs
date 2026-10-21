//! The `--version` handshake and the `AUTOZNG_SYSROOT` injection.
//!
//! Both are outward contracts with `cargo-autozng`: the handshake is how the frontend learns
//! which toolchain the driver needs and which capabilities it implements, and the sysroot is a
//! belt-and-braces second route to the compiler the driver already loaded.

use std::path::Path;
use std::process::Stdio;

use crate::harness;

#[test]
fn version_reports_identity_and_capabilities() {
    let output = harness::driver_command()
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn the driver");
    assert!(
        output.status.success(),
        "`--version` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or_default();
    assert!(
        first.starts_with("autozng-driver "),
        "unexpected handshake first line: {first:?}"
    );

    let rest: Vec<&str> = lines.collect();
    assert!(
        rest.iter()
            .any(|line| line.starts_with("pinned-toolchain: ")),
        "no `pinned-toolchain` line:\n{stdout}"
    );
    assert!(
        rest.iter()
            .any(|line| line.starts_with("built-against-rustc: rustc ")),
        "no `built-against-rustc` line:\n{stdout}"
    );

    let capabilities = rest
        .iter()
        .find_map(|line| line.strip_prefix("capabilities: "))
        .unwrap_or_else(|| panic!("no `capabilities` line:\n{stdout}"));
    for capability in ["output-zng", "out-dir", "output-dir-env", "sysroot-env"] {
        assert!(
            capabilities.split(", ").any(|cap| cap == capability),
            "the handshake does not advertise `{capability}`: {capabilities:?}"
        );
    }
}

#[test]
fn the_env_sysroot_is_injected_into_the_compiler_arguments() {
    // A correct sysroot is harmless: it is the same one the driver would derive anyway.
    let ok = wrapped_compile(Some(harness::toolchain()));
    assert!(
        ok.status.success(),
        "a correct `AUTOZNG_SYSROOT` should not break the compile:\n{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    // A bogus one proves the flag reaches rustc: without the injection the compile would
    // succeed from the loaded `librustc_driver`'s own sysroot.
    let broken = wrapped_compile(Some(&std::env::temp_dir()));
    assert!(
        !broken.status.success(),
        "a bogus `AUTOZNG_SYSROOT` should have made the compile fail, so it was not injected"
    );
}

/// Compile a tiny crate in wrapper mode, with `AUTOZNG_SYSROOT` set when `sysroot` is `Some`.
fn wrapped_compile(sysroot: Option<&Path>) -> std::process::Output {
    let scratch = harness::Scratch::new();
    harness::write_files(&scratch.path, &[("src/lib.rs", "pub struct Example;\n")]);

    let mut command = harness::driver_command();
    command
        .arg(harness::rustc_path())
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--crate-name=scratch_crate",
            "src/lib.rs",
        ])
        .current_dir(&scratch.path)
        .stdin(Stdio::null());
    if let Some(sysroot) = sysroot {
        command.env("AUTOZNG_SYSROOT", sysroot);
    }

    command.output().expect("failed to spawn the driver")
}
