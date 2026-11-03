//@ autozng_fail
//@ error_contains: `trait_path = "..."` needs an ADT or a type alias to an ADT
#[cfg_attr(autozng, autozng::rename(trait_path = "crate::Convert<u32>", suffix = "Cpp"))]
type Alias = u32;
