//@ autozng

// A selector whose argument does not match the rendered trait reference warns and leaves the
// method under its original name.

pub trait Convert<T> {
    fn convert(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Convert<u32>;
};

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

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = <Widget as Convert<u32>>::convert;
};
