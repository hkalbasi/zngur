//@ autozng

mod inner {
    #[cfg_attr(autozng, autozng::emit)]
    pub struct Inner;
}
