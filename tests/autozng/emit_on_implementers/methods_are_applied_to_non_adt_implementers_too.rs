//@ autozng

#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Describe {
    fn describe(&self) -> String;
}

impl Describe for str {
    fn describe(&self) -> String {
        self.to_owned()
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [&str; 0] = [];
