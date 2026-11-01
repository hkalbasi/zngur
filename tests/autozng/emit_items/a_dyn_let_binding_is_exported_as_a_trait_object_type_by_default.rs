//@ autozng

pub trait Producer<T> {
    fn produce(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _: &dyn Producer<u32>;
};
