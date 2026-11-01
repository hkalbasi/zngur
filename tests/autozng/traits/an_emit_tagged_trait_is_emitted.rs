//@ autozng

#[cfg_attr(autozng, autozng::emit)]
pub trait Store {
    fn put(&self, x: u64) -> u64;
    fn tag(&self, id: u64, name: &str);
    fn take(self) -> u32;
    fn stat() -> u8;
}
