//@ autozng_fail
//@ error_contains: `rename` on a type alias needs `method = "..."`
pub struct Renameable {
    pub value: u32,
}

impl Renameable {
    pub fn get(&self) -> u32 {
        self.value
    }
}

#[cfg_attr(autozng, autozng::rename("Alias2"))]
type Alias = Renameable;
