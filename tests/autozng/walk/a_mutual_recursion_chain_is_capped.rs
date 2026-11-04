//@ autozng
pub struct A {
    pub b: Option<Box<B>>,
}

pub struct B {
    pub a: Option<Box<A>>,
}
