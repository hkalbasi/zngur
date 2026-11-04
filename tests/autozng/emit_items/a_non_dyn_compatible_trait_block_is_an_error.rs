//@ autozng_fail
//@ error_contains: is not dyn compatible and cannot be exported as a `trait` block

#[cfg_attr(autozng, autozng::emit)]
pub trait Generic {
    fn get<T>(&self) -> T;
}
