//@ autozng_fail
//@ error_contains: `stack` layout align for `scratch_crate::WrongAlign` must match its real align 1 (found 4)
#[cfg_attr(autozng, autozng::layout(stack(size = 2, align = 4)))]
pub struct WrongAlign {
    pub a: u8,
    pub b: u8,
}
