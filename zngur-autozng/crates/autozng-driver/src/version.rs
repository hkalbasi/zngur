//! The driver's identity handshake.
//!
//! `cargo-autozng` runs `<driver> --version` to learn three things, before it decides how to
//! invoke the driver:
//!
//! * `pinned-toolchain` — the channel the driver was *meant* to be built with;
//! * `built-against-rustc` — the compiler that *actually* compiled it, which is the identity the
//!   frontend must match, because the driver loads `librustc_driver` from that compiler;
//! * `capabilities` — the frontend-facing features this driver implements, so the frontend can
//!   drive a driver whose command line has drifted.
//!
//! The output is one field per line so a value containing spaces stays unambiguous, and
//! `--version` is answered only outside wrapper mode, where the flag belongs to the real
//! compiler (see `crate::run`).

/// The driver's own version (the workspace version).
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The channel the driver was built for, baked by `build.rs` from `rust-toolchain.toml`.
pub const PINNED_TOOLCHAIN: &str = env!("AUTOZNG_KNOWN_GOOD_TOOLCHAIN");

/// The compiler that compiled the driver, baked by `build.rs` from `$RUSTC --version`.
pub const BUILT_AGAINST_RUSTC: &str = env!("AUTOZNG_BUILT_AGAINST_RUSTC");

/// The frontend-facing features this driver implements: the flags it accepts and the environment
/// variables it reads.
///
/// Capability names are a stable contract — an old name is never repurposed for a new meaning —
/// so a frontend written against a capability set keeps working with an older or newer driver.
/// Names are additive: a new capability is a new flag or variable an older driver simply does not
/// advertise.
pub const CAPABILITIES: &[&str] = &["output-zng", "out-dir", "output-dir-env", "sysroot-env"];

/// The name printed on the handshake's first line.
pub const NAME: &str = "autozng-driver";

/// Render the handshake, one field per line.
pub fn handshake() -> String {
    format!(
        "{NAME} {TOOL_VERSION}\n\
         pinned-toolchain: {PINNED_TOOLCHAIN}\n\
         built-against-rustc: {BUILT_AGAINST_RUSTC}\n\
         capabilities: {}\n",
        CAPABILITIES.join(", "),
    )
}
