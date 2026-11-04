//@ autozng

#[cfg_attr(autozng, autozng::rename("Renamed"))]
pub struct Original {
    pub value: u8,
}
