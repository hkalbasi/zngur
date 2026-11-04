//@ autozng

// A C++ class cannot have a field and a method with the same name, and `zngur` rejects the
// resulting `.zng`. The field is kept (it already exposes the value) and the colliding method is
// exposed as `width_method` instead.

pub struct Collides {
    pub width: u32,
}

impl Collides {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        0
    }
}
