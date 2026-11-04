//@ autozng

pub trait First {
    fn first(&self) -> u32;
}

pub trait Second {
    fn second(&self) -> u32;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: () = {
    // A trait object inside a container, and an auto-trait object, both name their principal
    // trait; the auto trait itself (`Send`) is not exported.
    let _: Option<&dyn First>;
    let _: Box<dyn Second>;
    let _: &(dyn First + Send);
};
