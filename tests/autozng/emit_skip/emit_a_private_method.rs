//@ autozng

pub struct Api;

impl Api {
    pub fn visible(&self) {}

    #[cfg_attr(autozng, autozng::emit)]
    fn hidden(&self) {}
}
