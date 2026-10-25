#[rustfmt::skip]
mod generated;

fn main() {
    let (ptr, length, capacity) = generated::make_tuple();
    assert_eq!((length, capacity), (17, 29));
    assert!(!ptr.is_null());
    assert_eq!(unsafe { *ptr }, 42);
}
