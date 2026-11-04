#[rustfmt::skip]
mod generated;

pub enum Choice {
    First { value: i32 },
    Second { value: i32 },
}

impl Choice {
    fn first() -> Self {
        Self::First { value: 42 }
    }

    fn second() -> Self {
        Self::Second { value: 24 }
    }
}

unsafe extern "C" {
    fn enum_variant_field_access_main();
}

fn main() {
    unsafe {
        enum_variant_field_access_main();
    }
}
