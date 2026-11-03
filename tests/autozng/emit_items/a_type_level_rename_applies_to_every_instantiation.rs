//@ autozng

// `rename(method = ...)` on the type renames an inherent method for every instantiation. A
// concrete `emit_items` site still wins for its own instantiation. The type-name `rename` in the
// first attribute is ignored (with a warning) but must not clobber the method rename.

#[cfg_attr(autozng, autozng::rename("IgnoredTypeName"))]
#[cfg_attr(autozng, autozng::rename(method = "get", suffix = "Cpp"))]
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
const _: (Generic<u32>, Generic<bool>) =
    (Generic { inner: 0 }, Generic { inner: true });

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(suffix = "U32")]
    let _ = Generic::<u32>::get;
};
