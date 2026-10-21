//! The well-known names the frontend uses: the cargo subcommand, the two executables, the
//! pinned channel, and the environment variables it reads and sets.
//!
//! The environment-variable names are a *contract* with `autozng-driver`, which declares the
//! same strings independently. The two crates must not depend on each other — the driver needs
//! `rustc_private` and the frontend is a plain binary — so the shared literal is duplicated
//! with a cross-reference on both sides rather than shared through a crate.

/// The cargo subcommand: `cargo autozng` invokes `cargo-autozng autozng …`.
pub const CARGO_SUBCOMMAND: &str = "autozng";

/// The driver executable's file name, without the platform suffix.
pub const DRIVER_BIN: &str = "autozng-driver";

/// The frontend executable's file name.
pub const FRONTEND_BIN: &str = "cargo-autozng";

/// The known-good toolchain channel this workspace builds drivers against, baked by `build.rs`
/// from `rust-toolchain.toml`.
pub const KNOWN_GOOD_TOOLCHAIN: &str = env!("AUTOZNG_KNOWN_GOOD_TOOLCHAIN");

/// Environment variable overriding the driver executable path.
///
/// Also what a test harness sets to point the frontend at a freshly built driver.
pub const ENV_DRIVER: &str = "AUTOZNG_DRIVER";

/// Environment variable overriding the autozng workspace root searched for a built driver.
pub const ENV_WORKSPACE: &str = "AUTOZNG_WORKSPACE";

/// Environment variable that skips the preflight and the toolchain forcing.
pub const ENV_NO_MANAGE: &str = "AUTOZNG_NO_MANAGE";

/// Environment variable overriding the toolchain channel resolved through rustup.
pub const ENV_TOOLCHAIN: &str = "AUTOZNG_TOOLCHAIN";

/// Environment variable naming the directory the `.zng` files are written to.
///
/// Set for the driver; the driver declares the same name (`autozng_driver::config`).
pub const ENV_OUTPUT_DIR: &str = "AUTOZNG_OUTPUT_DIR";

/// Environment variable naming an explicit `.zng` file. See [`ENV_OUTPUT_DIR`].
pub const ENV_OUTPUT_ZNG: &str = "AUTOZNG_OUTPUT_ZNG";

/// Environment variable naming the sysroot to pass to `rustc` as `--sysroot`.
///
/// Set for the driver; the driver declares the same name (`autozng_driver::config`).
pub const ENV_SYSROOT: &str = "AUTOZNG_SYSROOT";
