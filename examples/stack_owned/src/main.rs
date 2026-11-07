#[rustfmt::skip]
mod generated;

use generated::{BoxedInt, MyCppWrapper, OpaqueRef};

pub fn read_opaque(r: &OpaqueRef) -> i32 {
    r.read()
}

fn main() {
    let c = MyCppWrapper::new(5, 6);
    c.print();

    assert_eq!(std::mem::size_of::<OpaqueRef>(), 0);
    assert_eq!(generated::run_with_opaque_ref(), 7);

    let mut vec = Vec::new();
    vec.push(BoxedInt::new(42));
    assert_eq!(vec[0].get(), 42);
}
