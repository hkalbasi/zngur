//@ autozng

pub struct Public {
    pub value: u32,
}

mod inner {
    pub struct Hidden;
}

impl Public {
    pub fn take_hidden(&self, h: inner::Hidden) {}
    pub fn give(&self) -> u32 {
        0
    }
}
