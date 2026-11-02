//@ autozng
pub struct Holder;

impl Holder {
    pub fn borrowed(&self) -> &'static OnlyRef {
        Box::leak(Box::new(OnlyRef { value: 1 }))
    }
}

pub(crate) struct OnlyRef {
    pub value: u32,
}

impl OnlyRef {
    pub fn consume(self) {}
    pub fn peek(&self) {}
}
