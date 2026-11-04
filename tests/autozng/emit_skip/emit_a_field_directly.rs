//@ autozng

pub struct Direct {
    pub kept: u8,
    #[cfg_attr(autozng, autozng::emit)]
    forced: u16,
}
