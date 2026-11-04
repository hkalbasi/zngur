//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]
#![cfg_attr(autozng, autozng::walk_filter(exclude(External)))]

// `External` matches the owner's defining crate, so every non-local type reached from `Holder`
// keeps only its `type` block.

pub struct Holder {
    pub text: String,
    pub length: usize,
}
