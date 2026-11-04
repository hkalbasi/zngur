#[rustfmt::skip]
mod generated;

#[derive(Debug)]
struct RefOnly {
    a: i32,
}

impl RefOnly {
    fn get() -> &'static RefOnly {
        Box::leak(Box::new(RefOnly { a: 42 }))
    }

    fn get_mut() -> &'static mut RefOnly {
        Box::leak(Box::new(RefOnly { a: 0 }))
    }
}

unsafe extern "C" {
    fn only_by_ref_field_access_main();
}

fn main() {
    unsafe {
        only_by_ref_field_access_main();
    }
}
