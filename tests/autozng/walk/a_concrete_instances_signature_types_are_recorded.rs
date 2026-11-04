//@ autozng

/// A generic instance reached only through a field. Its methods' signatures are instantiated
/// with `char`, so `Inner<char>` and `Option<char>` need their own `type` blocks.
pub struct Holder {
    pub wrapped: Wrapper<char>,
}

pub struct Wrapper<T> {
    pub value: T,
}

impl<T> Wrapper<T> {
    pub fn inner(&self) -> Inner<T> {
        todo!()
    }
}

pub struct Inner<T> {
    pub item: T,
}

impl<T> Inner<T> {
    pub fn maybe(self) -> Option<T> {
        todo!()
    }
}
