//@ autozng

pub mod inner {
    pub struct Hidden;

    trait Secret {
        fn secret(&self);
    }

    impl Secret for Hidden {
        fn secret(&self) {}
    }

    #[cfg(autozng)]
    #[cfg_attr(autozng, autozng::emit_items)]
    const _: () = {
        let _ = <Hidden as Secret>::secret;
    };
}
