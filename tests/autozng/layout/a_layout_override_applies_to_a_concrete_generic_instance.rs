//@ autozng

#[cfg_attr(autozng, autozng::layout(heap_allocated))]
#[derive(Debug)]
pub struct Generic<T> {
    pub val: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: &[Generic<u32>] = &[];
