//@ autozng_fail
//@ error_contains: union field `integer` cannot be emitted

pub union Bits {
    #[cfg_attr(autozng, autozng::emit)]
    pub integer: u32,
    pub float: f32,
}
