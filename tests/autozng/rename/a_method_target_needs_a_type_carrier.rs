//@ autozng_fail
//@ error_contains: `method = "..."` is only valid on an ADT or a type alias
pub struct S;

impl S {
    #[cfg_attr(autozng, autozng::rename(method = "get", suffix = "Cpp"))]
    pub fn get(&self) {}
}
