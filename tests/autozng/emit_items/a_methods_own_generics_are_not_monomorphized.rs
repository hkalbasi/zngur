//@ autozng

pub struct Generic<T> {
    pub value: T,
}

impl<T> Generic<T> {
    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn with_other<D>(&self, _other: D) -> &T {
        &self.value
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Generic<i32>,); 0] = [];
