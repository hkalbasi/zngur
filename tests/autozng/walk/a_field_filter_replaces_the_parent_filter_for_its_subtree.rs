//@ autozng
pub struct Outer {
    #[cfg_attr(autozng, autozng::walk_filter(none))]
    pub inner: Inner,
}

pub(crate) struct Inner {
    pub value: u32,
}

impl Inner {
    pub fn method(&self) {}
}
