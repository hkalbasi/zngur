//@ autozng

mod top {
    pub mod inner {
        struct NotVisibleToRoot;
        pub struct PubInInner;
    }
    struct NotVisibleToRoot2;
}
