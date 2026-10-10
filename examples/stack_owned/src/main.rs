#[rustfmt::skip]
mod generated;

pub use generated::cpp::{MyConservativeWrapper, MyCppWrapper};

fn main() {
    let c = MyCppWrapper::new(5, 6);
    c.print();

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 32);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 16);

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();
}
