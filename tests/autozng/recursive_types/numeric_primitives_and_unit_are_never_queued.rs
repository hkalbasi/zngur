//@ autozng

pub struct Numbers;

impl Numbers {
    pub fn get(&self) -> u32 {
        0
    }

    pub fn set(&mut self, value: i64) {
        let _ = value;
    }

    pub fn nothing(&self) {}
}
