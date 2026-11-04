//@ autozng
#![cfg_attr(autozng, autozng::walk_filter(none))]

#[cfg_attr(autozng, autozng::emit)]
pub struct Explicit {
    pub value: u32,
}

impl Explicit {
    pub fn method(&self) {}
}
