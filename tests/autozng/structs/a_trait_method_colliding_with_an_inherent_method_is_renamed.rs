//@ autozng

// A trait method that shares its name with an inherent method is exposed as `{name}_{Trait}`, so
// both are callable from C++. The warning points at the trait method, with the inherent method as
// a note.

pub struct Widget {
    pub value: u32,
}

impl Widget {
    pub fn width(&self) -> u32 {
        self.value
    }
}

#[cfg_attr(autozng, autozng::emit)]
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Measure {
    fn width(&self) -> u32;
}

impl Measure for Widget {
    fn width(&self) -> u32 {
        self.value
    }
}
