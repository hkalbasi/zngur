//@ autozng

// `rename(trait_path = ...)` renames the trait's methods on the carrier type only, and `method = ...`
// narrows it to one method. Another implementer keeps the original names.

#[cfg_attr(autozng, autozng::emit)]
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Measure {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

#[cfg_attr(autozng, autozng::rename(trait_path = "crate::Measure", suffix = "Cpp"))]
pub struct Widget {
    pub value: u32,
}

impl Measure for Widget {
    fn width(&self) -> u32 {
        self.value
    }

    fn height(&self) -> u32 {
        self.value
    }
}

#[cfg_attr(
    autozng,
    autozng::rename(trait_path = "crate::Measure", method = "width", suffix = "Cpp")
)]
pub struct Panel {
    pub value: u32,
}

impl Measure for Panel {
    fn width(&self) -> u32 {
        self.value
    }

    fn height(&self) -> u32 {
        self.value
    }
}

pub struct Plain {
    pub value: u32,
}

impl Measure for Plain {
    fn width(&self) -> u32 {
        self.value
    }

    fn height(&self) -> u32 {
        self.value
    }
}
