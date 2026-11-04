//! Plumbing tests for `autozng-driver`.
//!
//! The declarative driver cases live in `tests/autozng` and run through `cargo xtask test`.
//! These tests exercise the handshake and wrapper-mode process behavior directly.

mod harness;

mod handshake;
mod wrapper;
