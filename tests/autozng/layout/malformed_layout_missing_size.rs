//@ autozng_fail
//@ error_contains: malformed `autozng::layout` attribute input
#[cfg_attr(autozng, autozng::layout(stack(align = 4)))]
pub struct MissingSize {
    pub a: i32,
}
