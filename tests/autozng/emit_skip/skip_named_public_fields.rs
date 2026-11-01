//@ autozng

#[cfg_attr(autozng, autozng::skip(dropped))]
pub struct Fields {
    pub kept: u8,
    pub dropped: u8,
}
