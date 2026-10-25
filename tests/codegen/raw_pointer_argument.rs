#[rustfmt::skip]
mod generated;

fn main() {
    let mut value = 42;
    assert_eq!(generated::read_const_pointer(&value), 42);
    assert_eq!(generated::read_mut_pointer(&mut value), 44);
    assert_eq!(generated::read_const_pointer(&value), 44);
}
