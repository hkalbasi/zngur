//@ autozng

fn twice<T: Clone>(value: T) -> (T, T) {
    (value.clone(), value)
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: () = {
    let _ = twice::<u8>;
};
