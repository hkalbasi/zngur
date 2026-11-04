//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]
#![cfg_attr(autozng, autozng::walk_filter(exclude(Crate("core"))))]

// `Ordering` is printed as `crate::Ordering` through its re-export, but it is defined in `core`.
// `Crate("core")` must therefore match it, while `PathPrefix("crate::...")` would match the
// re-export path.

pub use std::cmp::Ordering;

pub struct Holder {
    pub ordering: Ordering,
}
