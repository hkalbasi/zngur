//@ autozng

pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: &[(Container<i32>, Container<String>)] = &[];
