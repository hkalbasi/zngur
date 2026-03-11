#![feature(box_into_inner)]

use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Default, Debug)]
struct Example {
    inner: String,
}

type Alias = Example;
trait Trait {
    type Output;
}
impl Trait for Example {
    type Output = Example;
}

#[allow(unused)]
impl Example {
    pub fn by_value(self: Self) {
        self.by_ref("by_val");
    }

    pub fn by_ref(self: &Self, source: &str) {
        println!("{source}: {}", self.inner);
    }

    pub fn by_ref_mut(self: &mut Self) {
        self.inner = "by_ref_mut".to_string();
        self.by_ref("mut");
    }

    pub fn by_box(self: Box<Self>) {
        self.by_ref("by_box");
        Box::into_inner(self).by_value();
    }

    pub fn by_rc(self: Rc<Self>) {
        self.by_ref("by_rc");
    }

    pub fn by_arc(self: Arc<Self>) {
        self.by_ref("by_arc");
    }

    pub fn by_pin(self: Pin<&Self>) {
        self.by_ref("by_pin");
    }

    pub fn explicit_type(self: Arc<Example>) {
        self.by_ref("explicit");
    }

    pub fn with_lifetime<'a>(self: &'a Self) {
        self.by_ref("lifetime");
    }

    pub fn nested<'a>(self: &mut &'a Arc<Rc<Box<Alias>>>) {
        self.by_ref("nested");
    }

    pub fn via_projection(self: <Example as Trait>::Output) {
        self.by_ref("via_projection");
    }

    pub fn from(name: String) -> Self {
        Example { inner: name }
    }
}

fn main() {
    let example = Example::from("Hello".to_string());
    example.by_value();

    let boxed = Box::new(Example::default());
    boxed.by_box();

    Example::default().by_ref_mut();
    Example::default().with_lifetime();
}
