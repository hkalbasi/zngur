//@ autozng

pub struct S;

impl S {
    pub fn give<D>(&self, value: D) -> D {
        value
    }

    #[cfg(autozng)]
    #[cfg_attr(autozng, autozng::emit_items)]
    const MONO: () = {
        let _ = Self::give::<u8>;
        #[cfg_attr(autozng, autozng::rename("give_u32"))]
        let _ = Self::give::<u32>;
    };
}
