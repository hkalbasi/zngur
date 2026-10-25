#[rustfmt::skip]
mod generated;

mod nested {
    pub fn relative() -> i32 {
        20
    }
}

fn main() {
    assert_eq!(generated::verify(), 42);
}
