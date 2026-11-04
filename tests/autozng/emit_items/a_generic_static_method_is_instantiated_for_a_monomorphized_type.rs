//@ autozng

pub struct Wrapper<T> {
    pub foo: T,
}

impl<T> Wrapper<T> {
    pub fn make(foo: T) -> Self {
        Wrapper { foo }
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _: Wrapper<u32>;
};
