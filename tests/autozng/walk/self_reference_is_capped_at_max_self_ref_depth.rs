//@ autozng
pub struct Node {
    pub next: Option<Box<Node>>,
}

impl Node {
    pub fn next_ref(&self) -> Option<&Node> {
        self.next.as_deref()
    }
}
