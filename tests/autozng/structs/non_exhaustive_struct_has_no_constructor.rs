//@ autozng

#[non_exhaustive]
pub struct Info {
    pub width: u32,
    pub height: u32,
}

impl Info {
    pub fn new(width: u32, height: u32) -> Info {
        Info { width, height }
    }
}
