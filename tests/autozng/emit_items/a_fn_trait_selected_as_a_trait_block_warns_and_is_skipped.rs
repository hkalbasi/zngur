//@ autozng

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Fn(u32) -> bool;
};
