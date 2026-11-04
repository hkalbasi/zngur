//@ autozng
pub struct Holder;

impl Holder {
    pub fn borrowed(&self) -> &'static OnlyRef {
        Box::leak(Box::new(OnlyRef { value: 1 }))
    }

    pub fn owned(&self) -> OnlyRef {
        OnlyRef { value: 2 }
    }
}

pub(crate) struct OnlyRef {
    pub value: u32,
}

impl OnlyRef {
    pub fn consume(self) {}
    pub fn peek(&self) {}
}
