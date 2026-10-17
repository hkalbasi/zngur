pub struct Container<T> {
    values: [T; 32],
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Container<i32>, Container<u8>); 0] = [];
