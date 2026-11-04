//@ autozng_fail
//@ error_contains: `conservative` layout size for `scratch_crate::TooSmall` must be at least its real size 4 (found 2)
#[cfg_attr(autozng, autozng::layout(conservative(size = 2, align = 4)))]
pub struct TooSmall {
    pub a: i32,
}
