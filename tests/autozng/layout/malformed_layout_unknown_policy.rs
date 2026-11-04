//@ autozng_fail
//@ error_contains: malformed `autozng::layout` attribute input
#[cfg_attr(autozng, autozng::layout(unknown_policy))]
pub struct Unknown {
    pub a: i32,
}
