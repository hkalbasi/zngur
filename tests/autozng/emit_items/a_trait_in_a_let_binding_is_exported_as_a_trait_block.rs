//@ autozng

pub trait Producer<T> {
    fn produce(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Producer<u32>;
    let _: &dyn Producer<String>;
};
