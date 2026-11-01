//@ autozng

#[derive(Debug)]
pub struct Generic<T> {
    pub value: T,
}

pub struct NotDebug;

impl<T: std::fmt::Debug> Generic<T> {
    pub fn describe(&self) -> String {
        format!("{self:?}")
    }
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: [(Generic<i32>, Generic<NotDebug>); 0] = [];
