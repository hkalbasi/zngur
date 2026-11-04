//@ autozng_fail
//@ error_contains: union field `integer` cannot be emitted

#[cfg_attr(autozng, autozng::emit(integer))]
pub union Bits {
    pub integer: u32,
    pub float: f32,
}
