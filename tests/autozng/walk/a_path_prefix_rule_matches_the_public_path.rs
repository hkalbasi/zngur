//@ autozng
#![cfg_attr(
    autozng,
    autozng::walk_filter(exclude(PathPrefix("crate::nested")))
)]

pub mod nested {
    pub struct Hidden {
        pub value: u32,
    }
}

pub struct Visible {
    pub hidden: nested::Hidden,
}
