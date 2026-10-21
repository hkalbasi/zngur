//! Integration tests for `autozng-driver`.
//!
//! Each module snapshots the `.zng` the real driver produces for a small scratch crate that
//! exercises one slice of `autozng`'s functionality. See [`harness`] for the machinery.
//!
//! Snapshots use `expect-test`: run with
//! `UPDATE_EXPECT=1 cargo test -p autozng-driver --test integration` to regenerate them, then
//! review the diff.

mod harness;

mod emit_items;
mod emit_on_implementers;
mod emit_skip;
mod enums;
mod free_functions;
mod layout;
mod methods;
mod policy;
mod recursive_types;
mod structs;
mod traits;
mod visibility;
mod warnings;
mod wrapper;
