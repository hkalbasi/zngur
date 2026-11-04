//@ autozng

// A `method = "..."` that names no inherent method warns and leaves the type unchanged.

#[cfg_attr(autozng, autozng::rename(method = "missing", suffix = "Cpp"))]
pub struct NoSuchMethod {
    pub value: u32,
}
