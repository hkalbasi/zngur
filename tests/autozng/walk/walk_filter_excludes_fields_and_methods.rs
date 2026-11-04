//@ autozng
#[cfg_attr(
    autozng,
    autozng::walk_filter(exclude(Named("secret"), InherentMethods))
)]
pub struct Filtered {
    pub visible: u32,
    pub secret: u32,
}

impl Filtered {
    pub fn method(&self) {}
}
