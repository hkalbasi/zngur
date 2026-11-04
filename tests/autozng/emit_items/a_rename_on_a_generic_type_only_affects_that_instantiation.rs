//@ autozng

// A rename recorded for one instantiation of a generic type applies only to that instantiation.
// `char` is the control: it has no rename site and keeps the method's own name. Each instance
// still gets exactly one `get` method (no duplicates from the other sites).

pub struct Generic<T> {
    pub inner: T,
}

impl<T> Generic<T> {
    pub fn get(&self) -> &T {
        &self.inner
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: (Generic<u32>, Generic<bool>, Generic<char>) = (
    Generic { inner: 0 },
    Generic { inner: true },
    Generic { inner: 'a' },
);

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(suffix = "U32")]
    let _ = Generic::<u32>::get;
    #[autozng::rename(suffix = "Bool")]
    let _ = Generic::<bool>::get;
};
