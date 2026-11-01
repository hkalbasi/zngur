//@ autozng

pub enum Payload {
    Text(String),
    Number { value: i32, extra: i32 },
}
