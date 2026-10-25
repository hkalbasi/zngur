#[rustfmt::skip]
mod generated;

fn rust_safe() -> i32 {
    42
}

unsafe fn rust_unsafe() -> i32 {
    43
}

unsafe fn rust_legacy_unsafe() -> i32 {
    44
}

#[deny(unused_unsafe)]
fn main() {
    assert_eq!(generated::cpp_safe(), 42);
    assert_eq!(unsafe { generated::cpp_unsafe() }, 43);
    assert_eq!(generated::verify_rust_safety(), 1);
}
