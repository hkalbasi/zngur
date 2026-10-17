//! `emit_on_implementers`: emitting a trait's functions on every implementing type, without the
//! type having to enumerate them in an `emit_items` const.

use expect_test::expect;

use crate::harness::run;

#[test]
fn a_trait_declaration_emits_its_methods_on_every_implementer() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Greet {
    fn greet(&self) -> String;
}

pub struct Person {
    pub name: String,
}

impl Greet for Person {
    fn greet(&self) -> String {
        "hi".to_owned()
    }
}
"#);
    // `Person` implements `Greet`, so `greet` is attached to it with the trait as its `use_path`.
    // The trait itself is not `emit`-tagged, so no `trait` block is rendered.
    out.assert_zng(expect![[r#"
        type crate::Person {
            #layout(size = 24, align = 8);

            field name (offset = 0, type = ::std::string::String);

            fn greet(&self) -> ::std::string::String use crate::Greet;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn an_emit_tagged_trait_is_both_declared_and_applied() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit)]
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Greet {
    fn greet(&self) -> String;
}

pub struct Person;

impl Greet for Person {
    fn greet(&self) -> String {
        String::new()
    }
}
"#);
    // `emit` renders the trait block; `emit_on_implementers` additionally attaches `greet` to
    // `Person`.
    out.assert_zng(expect![[r#"
        type crate::Person {
            #layout(size = 0, align = 1);

            fn greet(&self) -> ::std::string::String use crate::Greet;
        }

        trait crate::Greet {
            fn greet(&self) -> ::std::string::String;
        }
    "#]]);
}

#[test]
fn methods_are_applied_to_non_adt_implementers_too() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Describe {
    fn describe(&self) -> String;
}

impl Describe for str {
    fn describe(&self) -> String {
        self.to_owned()
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [&str; 0] = [];
"#);
    // `str` is a non-ADT type; the trait method is attached to it just the same.
    out.assert_zng(expect![[r#"
        type str {
            wellknown_traits(?Sized, Debug);

            fn describe(&self) -> ::std::string::String use crate::Describe;
        }
    "#]]);
}

#[test]
fn a_generic_impl_block_emits_methods_on_matching_instantiations() {
    let out = run(r#"
pub trait Describe {
    fn describe(&self) -> String;
}

pub struct Wrapper<T> {
    pub value: T,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: std::fmt::Debug> Describe for Wrapper<T> {
    fn describe(&self) -> String {
        format!("{:?}", self.value)
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [Wrapper<u32>; 0] = [];
"#);
    // The impl block's `T` is resolved against the concrete `Wrapper<u32>` (and its `T: Debug`
    // clause holds), so the trait's method is emitted on the instantiation.
    out.assert_zng(expect![[r#"
        type crate::Wrapper<u32> {
            #layout(size = 4, align = 4);

            fn describe(&self) -> ::std::string::String use crate::Describe;
        }
    "#]]);
}

#[test]
fn an_impl_blocks_clauses_filter_which_types_receive_the_methods() {
    let out = run(r#"
pub trait Marker {}

pub struct Marked;
impl Marker for Marked {}

pub struct Unmarked;

pub trait Describe {
    fn describe(&self) -> String;
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: Marker> Describe for T {
    fn describe(&self) -> String {
        String::new()
    }
}
"#);
    // `Marked: Marker` holds, so it receives `describe`; `Unmarked` does not satisfy the impl's
    // clause and is left untouched.
    out.assert_zng(expect![[r#"
        type crate::Marked {
            #layout(size = 0, align = 1);

            fn describe(&self) -> ::std::string::String use crate::Describe;
        }

        type crate::Unmarked {
            #layout(size = 0, align = 1);
        }
    "#]]);
}

#[test]
fn an_explicitly_enumerated_method_is_not_duplicated() {
    let out = run(r#"
pub struct Thing {
    pub value: u32,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
pub trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Thing {
    fn describe(&self) -> String {
        String::new()
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = <Thing as Describe>::describe;
};
"#);
    // The method is reachable both through `emit_items` and `emit_on_implementers`; it is
    // emitted only once.
    out.assert_zng(expect![[r#"
        type crate::Thing {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);

            fn describe(&self) -> ::std::string::String use crate::Describe;
        }
    "#]]);
}

#[test]
fn a_private_trait_is_skipped_with_a_warning() {
    let out = run(r#"
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
"#);
    // A trait's visibility gates `emit_on_implementers`, exactly like `emit`: a private trait
    // (not `pub`/`pub(crate)`) is not recorded, so `Hidden` receives nothing. The attribute is
    // warned about rather than the skip being silent.
    out.assert_zng(expect![[r#"
        type crate::inner::Hidden {
            #layout(size = 0, align = 1);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: private traits and functions can be exposed in a zngur interface
         --> src/lib.rs:7:25
          |
        7 |     #[cfg_attr(autozng, autozng::emit_on_implementers)]
          |                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn emit_items_enumerating_a_private_trait_method_warns_and_skips_it() {
    let out = run(r#"
pub mod inner {
    pub struct Hidden;

    trait Secret {
        fn secret(&self);
    }

    impl Secret for Hidden {
        fn secret(&self) {}
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const _: () = {
        let _ = <Hidden as Secret>::secret;
    };
}
"#);
    // A method of a private trait cannot be exported (its path is not visible to the generated
    // interface): it is skipped, and the `let` binding that named it is warned about.
    out.assert_zng(expect![[r#"
        type crate::inner::Hidden {
            #layout(size = 0, align = 1);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: private traits and functions can be exposed in a zngur interface
          --> src/lib.rs:18:9
           |
        18 |         let _ = <Hidden as Secret>::secret;
           |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn emit_traits_enumerating_a_private_trait_warns_and_skips_it() {
    let out = run(r#"
mod inner {
    trait Secret<T> {
        fn secret(&self) -> T;
    }

    #[cfg(autozng)]
    #[autozng::emit_items(emit_traits = true)]
    const _: [&dyn Secret<u32>; 0] = [];
}
"#);
    // A private trait enumerated through `emit_traits = true` is skipped, but not silently: the
    // `emit_items` const is warned about.
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: private traits and functions can be exposed in a zngur interface
          --> src/lib.rs:11:5
           |
        11 |     const _: [&dyn Secret<u32>; 0] = [];
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn an_impl_whose_trait_argument_is_not_determined_by_self_is_skipped() {
    let out = run(r#"
pub trait Marker {}

pub struct Marked;
impl Marker for Marked {}

pub trait Describe<T> {
    fn describe(&self) -> String;
}

pub struct Foo {
    pub value: u32,
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
impl<T: Marker> Describe<T> for Foo {
    fn describe(&self) -> String {
        String::new()
    }
}
"#);
    // `T` is not determined by `Foo`'s `Self` type, so there is no concrete `Foo: Describe<T>`
    // to emit; the impl is skipped instead of producing an ill-formed `.zng` (or crashing).
    out.assert_zng(expect![[r#"
        type crate::Foo {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);
        }

        type crate::Marked {
            #layout(size = 0, align = 1);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}
