//@ autozng

pub struct Thing;

impl Thing {
    #[cfg_attr(autozng, autozng::rename("Renamed"))]
    pub fn original(&self) {}

    #[cfg_attr(autozng, autozng::rename(case = "Pascal"))]
    pub fn long_snake_name(&self) {}
}
