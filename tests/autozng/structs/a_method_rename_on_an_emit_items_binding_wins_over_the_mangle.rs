//@ autozng

// An explicit `rename` on the `emit_items` binding that names the method wins over the automatic
// `_method` mangle, so no collision warning is emitted.

pub struct Collides {
    pub width: u32,
}

impl Collides {
    pub fn width(&self) -> u32 {
        self.width
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(suffix = "Cpp")]
    let _ = Collides::width;
};
