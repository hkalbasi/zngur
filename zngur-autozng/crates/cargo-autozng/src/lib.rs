//! The `cargo-autozng` command frontend.
//!
//! `cargo autozng` runs `cargo check` with [`autozng-driver`](../autozng_driver/index.html)
//! installed as the rustc workspace wrapper, so every workspace member is analysed and a `.zng`
//! interface file is emitted per crate. It mirrors `cargo clippy` / `clippy-driver`.
//!
//! # Contract
//!
//! * **Analysis only.** The driver stops the compilation after the compiler's analysis phase, so
//!   no binaries or libraries are produced even though cargo reports success.
//! * **Nothing is cached.** No dep-info file is written, so the analysis runs on every
//!   invocation and `cargo autozng` is never reported as up to date.
//! * **The toolchain is the driver's.** The compiler forced for the wrapped build is the one the
//!   installed driver was built against — not the frontend's, and not the project's.

pub mod args;
pub mod config;
pub mod generate;
pub mod help;
pub mod launch;
pub mod run;
pub mod toolchain;
