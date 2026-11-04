//@ autozng

mod inner {
    trait Secret<T> {
        fn secret(&self) -> T;
    }

    #[cfg(autozng)]
    #[cfg_attr(autozng, autozng::emit_items(emit_traits = true))]
    const _: [&dyn Secret<u32>; 0] = [];
}
