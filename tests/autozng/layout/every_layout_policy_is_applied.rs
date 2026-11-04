//@ autozng

#[cfg_attr(autozng, autozng::layout(stack(size = 8, align = 4)))]
pub struct Stacked {
    pub a: i32,
    pub b: i32,
}

#[cfg_attr(autozng, autozng::layout(conservative(size = 16, align = 8)))]
pub struct Wide {
    pub a: i32,
}

#[cfg_attr(autozng, autozng::layout(heap_allocated))]
pub struct Heaped {
    pub a: i32,
}

#[cfg_attr(autozng, autozng::layout(only_by_ref))]
pub struct RefOnly {
    pub a: i32,
}
