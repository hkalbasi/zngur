//@ autozng

// A type implementing two instantiations of the same trait emits two methods with the same Rust
// name. The per-instantiation trait renames give them distinct C++ names, so the renames (not the
// automatic collision mangle) resolve the collision.

pub trait Convert<T> {
    fn convert(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Convert<u32>;
    let _: &dyn Convert<bool>;
};

#[cfg_attr(
    autozng,
    autozng::rename(trait_path = "crate::Convert<u32>", suffix = "U32")
)]
#[cfg_attr(
    autozng,
    autozng::rename(trait_path = "crate::Convert<bool>", suffix = "Bool")
)]
pub struct Widget {
    pub value: u32,
}

impl Convert<u32> for Widget {
    fn convert(&self) -> u32 {
        self.value
    }
}

impl Convert<bool> for Widget {
    fn convert(&self) -> bool {
        self.value != 0
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = <Widget as Convert<u32>>::convert;
    let _ = <Widget as Convert<bool>>::convert;
};
