//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]
#![cfg_attr(
    autozng,
    autozng::walk_filter(exclude(Crate("std"), Crate("core"), Crate("alloc")))
)]

// The local type keeps its interface; the standard-library types it mentions are declared but
// not walked, so they stay layout-only even with the external-interface policy on.

pub struct Holder {
    pub text: String,
    pub values: Vec<u32>,
}
