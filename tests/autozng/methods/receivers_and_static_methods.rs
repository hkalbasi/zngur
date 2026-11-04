//@ autozng

pub struct Counter {
    pub value: i32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { value: 0 }
    }

    pub fn get(&self) -> i32 {
        self.value
    }

    pub fn set(&mut self, value: i32) {
        self.value = value;
    }

    pub fn consume(self) -> i32 {
        self.value
    }

    pub fn add(a: i32, b: i32) -> i32 {
        a + b
    }
}
