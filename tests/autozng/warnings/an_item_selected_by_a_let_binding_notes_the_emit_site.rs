//@ autozng

pub struct S;

impl S {
    pub fn give<D: Copy>(&self, value: D) -> [D; 4] {
        [value; 4]
    }

    #[cfg(autozng)]
    #[cfg_attr(autozng, autozng::emit_items)]
    const MONO: () = {
        let _ = Self::give::<u8>;
    };
}
