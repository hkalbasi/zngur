//@ autozng

pub struct Wrapper<T> {
    pub tag: u8,
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Wrapper<i32>, Wrapper<String>); 0] = [];
