//@ autozng

// A type with a lifetime parameter is recorded once from its definition (which keeps `'a`) and
// once more from a signature (where regions are erased). Both identities must collapse to one
// `type` block.

pub struct Holder<'a> {
    pub value: &'a u32,
}

pub fn wrap<'a>(value: &'a u32) -> Holder<'a> {
    Holder { value }
}
