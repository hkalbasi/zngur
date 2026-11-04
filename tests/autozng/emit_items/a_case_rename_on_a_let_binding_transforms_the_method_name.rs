//@ autozng

pub struct S;

impl S {
    pub fn long_name<D>(&self, value: D) -> D {
        value
    }

    #[cfg(autozng)]
    #[cfg_attr(autozng, autozng::emit_items)]
    const MONO: () = {
        #[cfg_attr(autozng, autozng::rename(case = "Pascal"))]
        let _ = Self::long_name::<u8>;
    };
}
