//@ autozng
#![cfg_attr(
    autozng,
    autozng::walk_filter(
        exclude(Crate("std"), Crate("core"), Crate("alloc")),
        include(PathPrefix("crate::module"), External),
    )
)]

pub struct Holder {
    pub value: u32,
}
