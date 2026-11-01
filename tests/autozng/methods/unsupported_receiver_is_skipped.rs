//@ autozng

pub struct OnlyBoxable;

impl OnlyBoxable {
    pub fn by_box(self: Box<Self>) {}
    pub fn fine(&self) {}
}
