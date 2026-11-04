//@ autozng

#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Greet {
    fn greet(&self) -> String;
}

pub struct Person {
    pub name: String,
}

impl Greet for Person {
    fn greet(&self) -> String {
        "hi".to_owned()
    }
}
