//@ autozng

pub struct Payload {
    pub value: String,
}

pub fn make() -> Payload {
    Payload {
        value: String::new(),
    }
}

pub fn consume(payload: &Payload) -> Vec<u8> {
    let _ = payload;
    Vec::new()
}
