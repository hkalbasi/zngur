//@ autozng

#[cfg_attr(autozng, autozng::emit)]
pub trait Greeter<T: std::fmt::Debug> {
    fn greet(&self) -> T;
    fn count(&self) -> u32;
}
