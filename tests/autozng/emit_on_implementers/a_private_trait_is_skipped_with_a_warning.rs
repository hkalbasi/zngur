//@ autozng

pub mod inner {
    pub struct Hidden;

    #[cfg_attr(autozng, autozng::emit_on_implementers)]
    trait Secret {
        fn secret(&self);
    }

    impl Secret for Hidden {
        fn secret(&self) {}
    }
}
