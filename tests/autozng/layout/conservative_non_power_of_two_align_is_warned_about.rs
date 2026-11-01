//@ autozng

#[cfg_attr(autozng, autozng::layout(conservative(size = 4, align = 3)))]
pub struct FourBytes {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
}
