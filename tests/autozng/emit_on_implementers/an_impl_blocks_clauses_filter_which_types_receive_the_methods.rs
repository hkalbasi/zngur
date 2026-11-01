//@ autozng

pub trait Marker {}

pub struct Marked;
impl Marker for Marked {}

pub struct Unmarked;

pub trait Describe {
    fn describe(&self) -> String;
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: Marker> Describe for T {
    fn describe(&self) -> String {
        String::new()
    }
}
