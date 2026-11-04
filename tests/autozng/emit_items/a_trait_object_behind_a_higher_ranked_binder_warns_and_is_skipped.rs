//@ autozng

pub trait Callback {
    fn call_back(&self) -> u32;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    // The function pointer introduces a higher-ranked binder, so the trait object cannot be
    // stabilized and is warned about instead of named.
    let _: fn(&dyn Callback);
};
