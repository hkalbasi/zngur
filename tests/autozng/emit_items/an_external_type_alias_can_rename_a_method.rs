//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]

// An external type cannot carry the attribute, so a type alias names it. `method = ...` on the
// alias renames the method for every instantiation.

pub use std::cmp::Ordering;

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(method = "reverse", suffix = "Cpp")]
    type OrderingAlias = ::std::cmp::Ordering;
};
