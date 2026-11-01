//@ autozng

#[cfg_attr(autozng, autozng::emit)]
pub trait Store {
    fn put(&self, x: u64) -> u64;
    fn tag(&self, id: u64, name: &str);
    fn take(self) -> u32;
    // A static method keeps the trait dyn compatible only with an explicit `Self: Sized` bound.
    fn stat() -> u8
    where
        Self: Sized;
}
