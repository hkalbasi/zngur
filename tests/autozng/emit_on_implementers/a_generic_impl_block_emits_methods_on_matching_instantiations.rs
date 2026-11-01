//@ autozng

pub trait Describe {
    fn describe(&self) -> String;
}

pub struct Wrapper<T> {
    pub value: T,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: std::fmt::Debug> Describe for Wrapper<T> {
    fn describe(&self) -> String {
        format!("{:?}", self.value)
    }
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: [Wrapper<u32>; 0] = [];
