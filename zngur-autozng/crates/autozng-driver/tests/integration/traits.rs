//! Trait declarations: an `emit`-tagged `pub`/`pub(crate)` trait is recorded into the analysis
//! context and rendered as a `trait ... { .. }` block in `ZngurSpec::traits`.

use expect_test::expect;

use crate::harness::run;

#[test]
fn an_emit_tagged_trait_is_emitted() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit)]
pub trait Store {
    fn put(&self, x: u64) -> u64;
    fn tag(&self, id: u64, name: &str);
    fn take(self) -> u32;
    fn stat() -> u8;
}
"#);
    // The trait's methods are emitted with their Rust receiver (`&self`/`self`/none), in
    // declaration order.
    out.assert_zng(expect![[r#"
        type str {
            wellknown_traits(?Sized, Debug);
        }

        trait crate::Store {
            fn put(&self, u64) -> u64;
            fn tag(&self, u64, &str);
            fn take(self) -> u32;
            fn stat() -> u8;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_trait_without_emit_is_not_emitted() {
    let out = run(r#"
pub trait NotEmitted {
    fn ignored(&self);
}
"#);
    // A trait's implicit emit action is `None`: only an explicit `emit` selects it.
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_generic_trait_lists_its_type_parameters() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit)]
pub trait Greeter<T: std::fmt::Debug> {
    fn greet(&self) -> T;
    fn count(&self) -> u32;
}
"#);
    // The trait's own type parameters become `.zng` type variables. A method whose signature
    // mentions one (`greet` returns `T`) has no `.zng` spelling and is warn-and-skipped, while a
    // concrete method (`count`) is emitted.
    out.assert_zng(expect![[r#"
        trait crate::Greeter<T> {
            fn count(&self) -> u32;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping method `greet`: type parameters are not supported in a zngur interface
         --> src/lib.rs:6:5
          |
        6 |     fn greet(&self) -> T;
          |     ^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}
