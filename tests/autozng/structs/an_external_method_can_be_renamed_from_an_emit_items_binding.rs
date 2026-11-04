//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]

// An external method can be renamed from an `emit_items` binding: a non-generic method's `FnDef`
// has no generic args, so the rename is keyed by its `DefId` and reaches the re-exported method.

pub use std::cmp::Ordering;

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename("reversed")]
    let _ = Ordering::reverse;
};
