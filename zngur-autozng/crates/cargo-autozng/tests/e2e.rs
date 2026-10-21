//! End-to-end: the real `cargo-autozng` binary driving the real `autozng-driver` over a cargo
//! build, producing a `.zng` file for the project's primary test crate.
//!
//! This needs the `autozng-driver` and the pinned nightly with `rustc-dev`, so it **skips with a
//! message** when no driver can be found — set `AUTOZNG_DRIVER` (or build the driver in the
//! zngur-autozng workspace) to run it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The example crate this project uses as its primary manual test target.
const EXAMPLE_MANIFEST: &str = "examples/autozng_regression_test_1/Cargo.toml";

/// The crate name (and so the `.zng` file stem) of that example.
const EXAMPLE_CRATE: &str = "autozng_regression_test_1";

#[test]
fn generates_a_zng_file_for_the_example_crate() {
    let Some(driver) = find_driver() else {
        eprintln!(
            "skipping `generates_a_zng_file_for_the_example_crate`: no autozng-driver found \
             (set `AUTOZNG_DRIVER`, or build it in the zngur-autozng workspace)"
        );
        return;
    };

    let manifest = workspace_root().join(EXAMPLE_MANIFEST);
    assert!(
        manifest.is_file(),
        "missing example manifest `{}`",
        manifest.display()
    );

    let scratch = std::env::temp_dir()
        .join("cargo-autozng-e2e")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&scratch);
    let output_dir = scratch.join("zng");
    // A separate target directory from the one `cargo test` is using: cargo holds an exclusive
    // lock on its target directory for the whole test run, so a wrapped build sharing it would
    // deadlock.
    let target_dir = scratch.join("target");

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-autozng"))
        .arg("--driver")
        .arg(&driver)
        .arg("--output-dir")
        .arg(&output_dir)
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("failed to run `cargo-autozng`");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "`cargo-autozng` failed ({}):\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );

    let zng_path = output_dir.join(format!("{EXAMPLE_CRATE}.zng"));
    let zng = std::fs::read_to_string(&zng_path).unwrap_or_else(|err| {
        panic!(
            "no `.zng` at `{}`: {err}\n--- stderr ---\n{stderr}",
            zng_path.display()
        )
    });
    assert!(
        zng.contains("type crate::Example {"),
        "the generated `.zng` is missing the expected type block:\n{zng}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

/// The driver to run: `AUTOZNG_DRIVER`, else the workspace's built driver.
fn find_driver() -> Option<PathBuf> {
    if let Some(driver) = std::env::var_os("AUTOZNG_DRIVER") {
        let driver = PathBuf::from(driver);
        return driver.is_file().then_some(driver);
    }
    let located = cargo_autozng::launch::driver_path::locate();
    located.is_file().then_some(located)
}

/// The `zngur-autozng` workspace root, from the crate's own location.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
