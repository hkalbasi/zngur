#[rustfmt::skip]
mod generated;

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct class {
    pub int: i32,
}

impl class {
    pub fn xor(&self) -> i32 {
        self.int
    }
}

#[allow(non_camel_case_types)]
#[repr(u8)]
pub enum switch {
    class,
}

fn main() {
    assert_eq!(generated::and(), 41);
    assert_eq!(generated::verify_keywords(), 1);
}
