//@ autozng

// A concrete instantiation of a generic type can rename a colliding method: the type arguments go
// on the type path and the method's own arguments (if any) in a turbofish. The rename key covers
// the described-here args, which is what the renderer instantiates the method with.

pub struct Generic<T> {
    pub value: T,
}

impl<T> Generic<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: (Generic<u32>,) = (Generic { value: 0 },);

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(suffix = "Cpp")]
    let _ = Generic::<u32>::value;
};
