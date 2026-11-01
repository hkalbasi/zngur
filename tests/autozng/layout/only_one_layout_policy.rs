//@ autozng_fail
//@ error_contains: only one `autozng::layout` policy is allowed per type
#[cfg_attr(autozng, autozng::layout(heap_allocated))]
#[cfg_attr(autozng, autozng::layout(only_by_ref))]
pub struct Two {
    pub a: i32,
}
