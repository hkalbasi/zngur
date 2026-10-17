//! The `cargo-autozng` command frontend.
//!
//! This crate holds the logic of the `cargo-autozng` executable.
//! It's job is to run cargo with the [`autozng-driver`](../autozng_driver/index.html) as
//! the rustc wrapper. Similar to `cargo-clippy` and `cargo-driver`

pub mod run;
