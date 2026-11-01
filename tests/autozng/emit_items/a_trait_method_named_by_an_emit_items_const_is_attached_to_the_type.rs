//@ autozng

pub struct S;

impl std::fmt::Display for S {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S")
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = S::to_string;
};
