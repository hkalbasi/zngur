//! Test crate whose public API is a glob re-export of the external `png` crate, plus the smallest
//! entry point that gives C++ a concrete decoder.
//!
//! The glob exercises autozng's handling of re-exported *external* types: every re-exported item
//! the generated `.zng` names lives in `png`, not in this crate, so it is only reachable through
//! the re-export pass (`analyze::visit_reexported_items`).
//!
//! `open` exposes the concrete `Reader<BufReader<File>>`; the generic `Reader<R>` itself is not
//! emitted (a generic re-export has no concrete C++ type).

#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]

pub use png::*;

use std::fs::File;
use std::io::BufReader;

/// Open `path` and read its PNG header.
pub fn open(path: &str) -> Reader<BufReader<File>> {
    let file = File::open(path).expect("opening the test image");
    Decoder::new(BufReader::new(file))
        .read_info()
        .expect("reading the PNG header")
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    // for buffer building
    let _ = std::slice::from_raw_parts::<u8>;
    let _ = std::slice::from_raw_parts_mut::<u8>;

    // rename methods coliding with fields
    #[autozng::rename(prefix = "get_")]
    let _ = png::Info::animation_control;
    #[autozng::rename(prefix = "get_")]
    let _ = png::Info::frame_control;
};

#[cfg(not(autozng))]
pub mod generated;
