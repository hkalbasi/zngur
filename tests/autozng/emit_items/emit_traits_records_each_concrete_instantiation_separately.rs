//@ autozng

pub trait GenericTrait<T> {
    fn get_value(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: [&dyn GenericTrait<u32>; 0] = [];

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: [&dyn GenericTrait<u8>; 0] = [];
