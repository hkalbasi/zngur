//! The `cargo-autozng` command frontend.
//!
//! This crate holds the logic of the `cargo-autozng` executable.
//! It's job is to run cargo with the [`autozng-driver`](../autozng_driver/index.html) as
//! the rustc wrapper. Similar to `cargo-clippy` and `cargo-driver`
//!
//! The driver understands `--output-zng <path>` (explicit `.zng` file) and honours rustc's
//! `--out-dir`; `cargo-autozng` is responsible for forwarding that information to the driver
//! once the wrapper plumbing exists (see [`run`]).

pub mod run;
