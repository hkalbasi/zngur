//@ autozng

pub struct Thing {
    pub value: u32,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Thing {
    fn describe(&self) -> String {
        String::new()
    }
}

#[cfg(autozng)]
#[cfg_attr(autozng, autozng::emit_items)]
const _: () = {
    let _ = <Thing as Describe>::describe;
};
