//! Which items the default (`pub`) emission policy picks up, and how their path is rendered.

use expect_test::expect;

use crate::harness::run;

#[test]
fn only_fully_public_items_are_emitted() {
    let out = run(r#"
pub struct Public;
struct Private;
"#);
    out.assert_zng(expect![[r#"
        type crate::Public {
            #layout(size = 0, align = 1);
        }
    "#]]);
}

#[test]
fn crate_restricted_items_are_not_emitted_by_default() {
    let out = run(r#"
pub(crate) struct CrateVisible;

mod inner {
    pub(super) struct SuperVisible;
}
"#);
    out.assert_zng(expect![[r#""#]]);
}

#[test]
fn public_items_inside_private_modules_are_treated_as_private() {
    let out = run(r#"
mod inner {
    pub struct Inner;
}
"#);
    // A `pub` item whose module chain is not `pub` is not reachable from the crate root's public
    // interface: it is treated as private and not emitted by default.
    out.assert_zng(expect![[r#""#]]);
}

#[test]
fn emit_on_a_private_item_warns() {
    let out = run(r#"
mod inner {
    #[cfg_attr(autozng, autozng::emit)]
    pub struct Inner;
}
"#);
    // `pub struct Inner` behind a private module is not visible from the crate root, so the
    // explicit `emit` cannot expose it; the attribute is warned about rather than skipped
    // silently.
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: private traits and functions can be exposed in a zngur interface
         --> src/lib.rs:5:25
          |
        5 |     #[cfg_attr(autozng, autozng::emit)]
          |                         ^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn a_method_using_a_private_type_is_skipped() {
    let out = run(r#"
pub struct Public {
    pub value: u32,
}

mod inner {
    pub struct Hidden;
}

impl Public {
    pub fn take_hidden(&self, h: inner::Hidden) {}
    pub fn give(&self) -> u32 {
        0
    }
}
"#);
    // `inner::Hidden` is not visible from the crate root, so `take_hidden` cannot be emitted: the
    // method is skipped with a warning while `give` is emitted normally.
    out.assert_zng(expect![[r#"
        type crate::Public {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);

            fn give(&self) -> u32;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping method `take_hidden`: type `scratch_crate::inner::Hidden` is not visible from the crate root
          --> src/lib.rs:13:5
           |
        13 |     pub fn take_hidden(&self, h: inner::Hidden) {}
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn a_field_using_a_private_type_is_skipped() {
    let out = run(r#"
mod inner {
    pub struct Hidden;
}

pub struct Public {
    pub value: u32,
    pub hidden: inner::Hidden,
}
"#);
    // The field whose type is not visible from the crate root is skipped with a warning; the
    // public field is kept.
    out.assert_zng(expect![[r#"
        type crate::Public {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping field `hidden` of `scratch_crate::Public`: type `scratch_crate::inner::Hidden` is not visible from the crate root
          --> src/lib.rs:10:5
           |
        10 |     pub hidden: inner::Hidden,
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn items_not_reachable_from_the_crate_root_are_private() {
    let out = run(r#"
mod top {
    pub mod inner {
        struct NotVisibleToRoot;
        pub struct PubInInner;
    }
    struct NotVisibleToRoot2;
}
"#);
    // `top` is private, so nothing below it is visible from the crate root: a private item in a
    // nested module and a `pub` item behind a non-`pub` module alike are treated as private.
    out.assert_zng(expect![[r#""#]]);
}

#[test]
fn nested_module_paths_are_preserved() {
    let out = run(r#"
pub mod outer {
    pub mod inner {
        pub struct Deep;
    }
}
"#);
    out.assert_zng(expect![[r#"
        type crate::outer::inner::Deep {
            #layout(size = 0, align = 1);
        }
    "#]]);
}

#[test]
fn reexports_are_emitted_under_their_visible_paths() {
    // The shapes a crate uses to expose its API: private modules re-exported with `pub use`
    // (single, glob, renamed, chained, through a `pub(crate)` module), a re-export inside a `pub`
    // module, a value-namespace (function) re-export, and the two shapes that must *not* be
    // emitted (`pub` never re-exported, `pub(super)`).
    let out = run(r#"
mod a {
    pub struct A;
}
pub use a::A;

mod b {
    pub struct B;
}
pub use b::*;

pub(crate) mod c {
    pub struct C;
}
pub use c::C;

mod d {
    pub struct D;
}
pub use d::D as DRenamed;

mod e {
    pub struct E;
}
mod f {
    pub use crate::e::E;
}
pub use f::E;

mod g {
    pub struct G;
}
pub mod h {
    pub use crate::g::G;
}

mod i {
    pub fn i_fn() {}
}
pub use i::i_fn;

mod j {
    pub struct J;
}

mod l {
    pub(super) struct L;
}
"#);
    out.assert_zng(expect![[r#"
        type crate::A {
            #layout(size = 0, align = 1);
        }

        type crate::B {
            #layout(size = 0, align = 1);
        }

        type crate::C {
            #layout(size = 0, align = 1);
        }

        type crate::DRenamed {
            #layout(size = 0, align = 1);
        }

        type crate::E {
            #layout(size = 0, align = 1);
        }

        type crate::h::G {
            #layout(size = 0, align = 1);
        }

        fn crate::i_fn();
    "#]]);
}
