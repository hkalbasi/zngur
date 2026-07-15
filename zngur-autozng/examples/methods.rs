#![feature(register_tool)]
#![register_tool(autozng)]

use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

/// Docs for Example type
///   multiple lines
///     multiple attrs
///
/// <!--
/// autozng::emit(second)
/// -->
///
#[autozng::emit(third, forth)]
#[derive(Default, Debug)]
pub struct Example {
    /// autozng::rename(inner)
    pub first: String,
    second: i32,
    third: u32,
    forth: Vec<String>,
}

/// Alias of Example
type Alias = Example;

pub enum EnumEx {
    V1,
    V2(i32),
    V3(u32, u32),
    V4 { f1: u32 },
    V5 { f1: u32, f2: u32 },
}

/// a Trait
///   multiline doc comment
pub trait Trait {
    type Output;
    fn trait_fn(&self);
}

/// trait impl
impl Trait for Example {
    type Output = Example;
    fn trait_fn(&self) {
        self.by_ref("from_trait");
    }
}

/// main impl block
#[allow(unused)]
impl Example {
    /// by value fn
    pub fn by_value(self: Self) {
        self.by_ref("by_val");
    }

    /// by ref fn
    pub fn by_ref(self: &Self, source: &str) {
        println!("{source}: {}", self.first);
    }

    /// by ref mut fn
    pub fn by_ref_mut(self: &mut Self) {
        self.first = "by_ref_mut".to_string();
        self.by_ref("mut");
    }

    /// by box fn
    pub fn by_box(self: Box<Self>) {
        self.by_ref("by_box");
    }

    /// by rc fn
    pub fn by_rc(self: Rc<Self>) {
        self.by_ref("by_rc");
    }

    /// by arc fn
    pub fn by_arc(self: Arc<Self>) {
        self.by_ref("by_arc");
    }

    /// by pin fn
    pub fn by_pin(self: Pin<&Self>) {
        self.by_ref("by_pin");
    }

    /// by explicit type fn
    pub fn explicit_type(self: Arc<Example>) {
        self.by_ref("explicit");
    }

    /// fn with lifetime
    pub fn with_lifetime<'a>(self: &'a Self) {
        self.by_ref("lifetime");
    }

    /// fn with nested type
    pub fn nested<'a>(self: &mut &'a Arc<Rc<Box<Alias>>>) {
        self.by_ref("nested");
    }

    /// fn with type by projection
    pub fn via_projection(self: <Example as Trait>::Output) {
        self.by_ref("via_projection");
    }

    /// from ctor
    pub fn from(name: String) -> Self {
        Example {
            first: name,
            second: -1,
            third: 1,
            forth: vec!["hello".into(), "world".into()],
        }
    }
}

/// main entry fn
/// the mir of this should be emited
fn main() {
    let example = Example::from("Hello".to_string());
    example.by_value();

    let boxed = Box::new(Example::default());
    boxed.by_box();

    Example::default().by_ref_mut();
    Example::default().with_lifetime();
}
