//@ autozng

mod inner {
    pub trait Secret {
        fn secret(&self) -> u32;
    }
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn inner::Secret;
};
