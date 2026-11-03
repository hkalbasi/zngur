//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]

// Selecting an inherent method through `emit_items` records the re-exported type before the
// re-export pass. That partial record must not make the type look fully walked: the re-export
// still has to emit the variants and the remaining methods.

pub use std::cmp::Ordering;

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = Ordering::is_lt;
};
