//! Test crate whose entire public API is a glob re-export of the external `png` crate.
//!
//! It exists to exercise autozng's handling of re-exported *external* types: every public item
//! the generated `.zng` names lives in `png`, not in this crate, so it is only reachable through
//! the re-export pass (`analyze::visit_reexported_items`).

#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
// this is what *really* turns on the export of the png crate interface, without it only the
// types and their layout are emited
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]

pub use png::*;
