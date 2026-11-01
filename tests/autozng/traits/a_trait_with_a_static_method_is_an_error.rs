//@ autozng_fail
//@ error_contains: has a static method

#[cfg_attr(autozng, autozng::emit)]
pub trait Store {
    fn put(&self, x: u64) -> u64;
    fn stat() -> u8
    where
        Self: Sized;
}
