//@ autozng
pub trait Marker<T> {
    fn ping(&self);
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Marker<Vec<u32>>;
};
