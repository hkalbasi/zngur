//@ autozng

#[cfg_attr(autozng, autozng::emit)]
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Greet {
    fn greet(&self) -> String;
}

pub struct Person;

impl Greet for Person {
    fn greet(&self) -> String {
        String::new()
    }
}
