//@ autozng

pub struct Root {
    pub child: Child,
}

pub(crate) struct Child {
    pub value: u32,
}

impl Child {
    pub fn sibling(&self) -> Sibling {
        Sibling { value: 0 }
    }
}

pub(crate) struct Sibling {
    pub value: u32,
}

impl Sibling {
    pub fn back(&self) -> Child {
        Child { value: 0 }
    }
}
