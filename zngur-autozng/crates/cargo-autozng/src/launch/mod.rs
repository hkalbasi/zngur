//! Launching the wrapped cargo build: the loader path, driver discovery, the preflight and the
//! cargo command itself.

pub mod command;
pub mod driver_path;
pub mod dylib;
pub mod preflight;
pub mod target_dir;
