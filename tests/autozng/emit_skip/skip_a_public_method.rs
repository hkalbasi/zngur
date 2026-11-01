//@ autozng

pub struct Api;

impl Api {
    pub fn kept(&self) {}

    #[cfg_attr(autozng, autozng::skip)]
    pub fn dropped(&self) {}
}
