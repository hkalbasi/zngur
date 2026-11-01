//@ autozng

pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items(emit_traits = false))]
const _: [Container<u8>; 0] = [];
