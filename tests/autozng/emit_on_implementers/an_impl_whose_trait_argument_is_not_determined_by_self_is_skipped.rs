//@ autozng

pub trait Marker {}

pub struct Marked;
impl Marker for Marked {}

pub trait Describe<T> {
    fn describe(&self) -> String;
}

pub struct Foo {
    pub value: u32,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: Marker> Describe<T> for Foo {
    fn describe(&self) -> String {
        String::new()
    }
}
