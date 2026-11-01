//@ autozng

pub trait GenericTrait<T> {
    fn get_value(&self) -> T;
    fn count(&self) -> u32;
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items(emit_traits = true))]
const _: [&dyn GenericTrait<u32>; 0] = [];
