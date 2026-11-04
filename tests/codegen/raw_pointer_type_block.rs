#[rustfmt::skip]
mod generated;

pub struct Point {
    _x: u8,
}

fn main() {
    let (pointer, length) = generated::make_tuple();
    assert_eq!(length, 7);
    assert!(pointer.is_null());

    let (pointer, length) = generated::make_mut_tuple();
    assert_eq!(length, 9);
    assert!(pointer.is_null());
}
