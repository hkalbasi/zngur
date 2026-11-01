//@ autozng

pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: [(Container<i32>, Container<u8>); 0] = [];
