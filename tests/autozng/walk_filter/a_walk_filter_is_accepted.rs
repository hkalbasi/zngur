//@ autozng
#![cfg_attr(
    autozng,
    autozng::walk_filter(
        exclude(TraitMethods("::core::fmt::Debug")),
        include(Fields, InherentMethods),
    )
)]

#[cfg_attr(
    autozng,
    autozng::walk_filter(include(Fields), exclude(Named("ignored")))
)]
pub struct Holder {
    pub value: u32,
    #[cfg_attr(autozng, autozng::walk_filter(none))]
    pub ignored: u32,
}
