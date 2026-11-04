//@ autozng
#![cfg_attr(
    autozng,
    autozng::policy(
        implicitly_emit_external_interfaces,
        max_walk_depth = 8,
        max_external_depth = 4,
        max_trait_impl_depth = 4,
        max_trait_interface_depth = 2,
        max_self_ref_depth = 2,
    )
)]

pub struct Holder {
    pub value: u32,
}
