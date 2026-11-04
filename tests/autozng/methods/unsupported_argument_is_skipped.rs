//@ autozng

pub struct Args;

impl Args {
    pub fn ok(&self) {}

    pub fn takes_array(&self, values: [u8; 4]) {
        let _ = values;
    }
}
