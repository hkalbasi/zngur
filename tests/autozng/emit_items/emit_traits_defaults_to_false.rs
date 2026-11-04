//@ autozng

pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = false)]
const _: [Container<u8>; 0] = [];
