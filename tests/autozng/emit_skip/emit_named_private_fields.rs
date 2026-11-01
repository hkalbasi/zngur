//@ autozng

#[cfg_attr(autozng, autozng::emit(second, third))]
pub struct Selection {
    pub first: u8,
    second: u16,
    third: u32,
}
