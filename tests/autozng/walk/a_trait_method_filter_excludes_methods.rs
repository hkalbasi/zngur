//@ autozng
#[cfg_attr(autozng, autozng::walk_filter(exclude(TraitMethods)))]
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Ping {
    fn ping(&self);
}

#[cfg_attr(autozng, autozng::emit)]
pub struct Foo;

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl Ping for Foo {
    fn ping(&self) {}
}
