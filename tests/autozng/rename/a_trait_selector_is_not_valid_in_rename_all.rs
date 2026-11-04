//@ autozng_fail
//@ error_contains: `trait_path` is not supported in `rename_all`
#[cfg_attr(
    autozng,
    autozng::rename_all(trait_path = "crate::Convert<u32>", suffix = "Cpp")
)]
pub struct S {
    pub value: u32,
}
