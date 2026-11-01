//@ autozng_fail
//@ error_contains: `conservative` layout align for `scratch_crate::TooSmall` must be at least its real align 4 (found 2)
#[cfg_attr(autozng, autozng::layout(conservative(size = 4, align = 2)))]
pub struct TooSmall {
    pub a: i32,
}
