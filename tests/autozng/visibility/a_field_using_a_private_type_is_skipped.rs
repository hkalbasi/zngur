//@ autozng

mod inner {
    pub struct Hidden;
}

pub struct Public {
    pub value: u32,
    pub hidden: inner::Hidden,
}
