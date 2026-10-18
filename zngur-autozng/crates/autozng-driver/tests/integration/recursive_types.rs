//! Recursive interface recording: the types an emitted field or function signature exposes get
//! their own `type` block, as if they carried an explicit `emit`.
//!
//! A type exposed this way is queued for a third analysis pass (drained in `analyze`); visiting
//! it may expose further types. Numeric primitives and the unit tuple are never queued, a
//! private type is not queued (the field / function that exposed it is skipped instead), and a
//! type is never visited twice.

use expect_test::expect;

use crate::harness::run;

#[test]
fn interface_types_are_recorded_recursively() {
    let out = run(r#"
pub struct MyType {
    pub field1: String,
}

impl MyType {
    pub fn do_stuff(&self, data: Vec<u32>) -> bool {
        let _ = data;
        false
    }

    pub fn do_other_stuff(&self) -> u32 {
        0
    }
}
"#);
    // `String` (the field's type), `Vec<u32>` and `bool` are exposed by `MyType`'s interface, so
    // each gets its own `type` block; the numeric `u32` does not.
    out.assert_zng(expect![[r#"
        type ::std::string::String {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);
        }

        type ::std::vec::Vec<u32> {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);
        }

        type bool {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);
        }

        type crate::MyType {
            #layout(size = 24, align = 8);

            field field1 (offset = 0, type = ::std::string::String);

            fn do_stuff(&self, ::std::vec::Vec<u32>) -> bool;
            fn do_other_stuff(&self) -> u32;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_type_reached_through_an_interface_is_emitted_even_when_not_public() {
    let out = run(r#"
pub struct Root {
    pub child: Child,
}

pub(crate) struct Child {
    pub value: u32,
}

impl Child {
    pub fn sibling(&self) -> Sibling {
        Sibling { value: 0 }
    }
}

pub(crate) struct Sibling {
    pub value: u32,
}

impl Sibling {
    pub fn back(&self) -> Child {
        Child { value: 0 }
    }
}
"#);
    // `Child` is only `pub(crate)`, so the default policy would not emit it; it is pulled in
    // because `Root` exposes it, and its own method exposes `Sibling`. `Sibling`'s method points
    // back at `Child`, which has already been seen: the queue terminates.
    out.assert_zng(expect![[r#"
        type crate::Child {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);

            fn sibling(&self) -> crate::Sibling;
        }

        type crate::Root {
            #layout(size = 4, align = 4);

            field child (offset = 0, type = crate::Child);
        }

        type crate::Sibling {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = u32);

            fn back(&self) -> crate::Child;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_private_type_is_not_queued_and_its_field_is_skipped() {
    let out = run(r#"
pub struct Root {
    pub visible: u8,
    pub hidden: inner::Hidden,
}

mod inner {
    pub struct Hidden;
}
"#);
    // `inner::Hidden` is not visible from the crate root, so it is not queued: the generated code
    // cannot name it. The field that exposed it is skipped with a warning (the type is *not*
    // silently emitted under a broken path).
    out.assert_zng(expect![[r#"
        type crate::Root {
            #layout(size = 1, align = 1);

            field visible (offset = 0, type = u8);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping field `hidden` of `scratch_crate::Root`: type `scratch_crate::inner::Hidden` is not visible from the crate root
         --> src/lib.rs:6:5
          |
        6 |     pub hidden: inner::Hidden,
          |     ^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn numeric_primitives_and_unit_are_never_queued() {
    let out = run(r#"
pub struct Numbers;

impl Numbers {
    pub fn get(&self) -> u32 {
        0
    }

    pub fn set(&mut self, value: i64) {
        let _ = value;
    }

    pub fn nothing(&self) {}
}
"#);
    // A numeric primitive is a native C++ type and `()` is specialised by zngur: neither is a
    // `type` block, so the interface exposes no additional types.
    out.assert_zng(expect![[r#"
        type crate::Numbers {
            #layout(size = 0, align = 1);

            fn get(&self) -> u32;
            fn set(&mut self, i64);
            fn nothing(&self);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn free_function_interface_types_are_recorded() {
    let out = run(r#"
pub struct Payload {
    pub value: String,
}

pub fn make() -> Payload {
    Payload {
        value: String::new(),
    }
}

pub fn consume(payload: &Payload) -> Vec<u8> {
    let _ = payload;
    Vec::new()
}
"#);
    // `Payload` is public and already emitted; `String` and `Vec<u8>` are exposed by the free
    // functions' signatures and get their own blocks (`u8` is numeric and is not queued).
    out.assert_zng(expect![[r#"
        type ::std::string::String {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);
        }

        type ::std::vec::Vec<u8> {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);
        }

        type crate::Payload {
            #layout(size = 24, align = 8);

            field value (offset = 0, type = ::std::string::String);
        }

        fn crate::consume(&crate::Payload) -> ::std::vec::Vec<u8>;
        fn crate::make() -> crate::Payload;
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}
