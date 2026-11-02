//@ autozng
#[cfg_attr(autozng, autozng::emit)]
pub struct Foo;

#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Marker<T> {
    fn ping(&self);
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl Marker<Vec<u8>> for Foo {
    fn ping(&self) {}
}
