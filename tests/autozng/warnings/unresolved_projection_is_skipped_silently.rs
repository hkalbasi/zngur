//@ autozng

pub trait HasAssoc {
    type Output;
}

impl HasAssoc for u32 {
    type Output = i64;
}

struct Holder<T> {
    v: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Holder<<u32 as HasAssoc>::Output>,); 0] = [];
