//@ autozng

pub struct MyType {
    pub field1: String,
}

impl MyType {
    pub fn do_stuff(&self, data: Vec<u32>) -> bool {
        let _ = data;
        false
    }

    pub fn do_other_stuff(&self) -> u32 {
        0
    }
}
