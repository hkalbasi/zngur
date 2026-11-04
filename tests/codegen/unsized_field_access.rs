#[rustfmt::skip]
mod generated;

#[repr(C)]
struct UnsizedImpl<T: ?Sized> {
    value: i32,
    tail: T,
}

type Unsized = UnsizedImpl<[u8]>;

fn main() {
    let boxed: Box<UnsizedImpl<[u8; 8]>> = Box::new(UnsizedImpl {
        value: 42,
        tail: [0u8; 8],
    });
    let mut boxed: Box<Unsized> = boxed;
    generated::unsized_field_access_main(&mut *boxed);
}
