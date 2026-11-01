//@ autozng

#[derive(Debug)]
pub struct Generic<T> {
    pub value: T,
}

impl<T: std::fmt::Debug> Generic<T> {
    pub fn describe(&self) -> String {
        format!("{self:?}")
    }
}

impl<T: Clone + std::fmt::Debug> Generic<T> {
    pub fn clone_inner(&self) -> T {
        self.value.clone()
    }
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: [(Generic<i32>, Generic<String>); 0] = [];
