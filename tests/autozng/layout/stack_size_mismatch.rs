//@ autozng_fail
//@ error_contains: `stack` layout size for `scratch_crate::WrongSize` must match its real size 8 (found 4)
#[cfg_attr(autozng, autozng::layout(stack(size = 4, align = 4)))]
pub struct WrongSize {
    pub a: i32,
    pub b: i32,
}
