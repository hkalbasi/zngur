//@ autozng

pub struct Root {
    pub visible: u8,
    pub hidden: inner::Hidden,
}

mod inner {
    pub struct Hidden;
}
