//! `emit_items`: forcing concrete monomorphizations of a generic type into the output.

use expect_test::expect;

use crate::harness::run;

#[test]
fn monomorphize_a_generic_struct() {
    let out = run(r#"
pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Container<i32>, Container<u8>); 0] = [];
"#);
    out.assert_zng(expect![[r#"
        type crate::Container<i32> {
            #layout(size = 4, align = 4);
        }

        type crate::Container<u8> {
            #layout(size = 1, align = 1);
        }
    "#]]);
}

#[test]
fn monomorphization_keeps_the_type_layout() {
    let out = run(r#"
pub struct Wrapper<T> {
    pub tag: u8,
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Wrapper<i32>, Wrapper<String>); 0] = [];
"#);
    out.assert_zng(expect![[r#"
        type crate::Wrapper<::std::string::String> {
            #layout(size = 32, align = 8);
        }

        type crate::Wrapper<i32> {
            #layout(size = 8, align = 4);
        }
    "#]]);
}

#[test]
fn methods_are_instantiated_for_a_monomorphized_type() {
    let out = run(r#"
#[derive(Debug)]
pub struct Generic<T> {
    pub value: T,
}

impl<T: std::fmt::Debug> Generic<T> {
    pub fn describe(&self) -> String {
        format!("{self:?}")
    }
}

impl<T: Clone + std::fmt::Debug> Generic<T> {
    pub fn clone_inner(&self) -> T {
        self.value.clone()
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Generic<i32>, Generic<String>); 0] = [];
"#);
    out.assert_zng(expect![[r#"
        type crate::Generic<::std::string::String> {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);

            fn describe(&self) -> ::std::string::String;
            fn clone_inner(&self) -> ::std::string::String;
        }

        type crate::Generic<i32> {
            #layout(size = 4, align = 4);
            wellknown_traits(Debug);

            fn describe(&self) -> ::std::string::String;
            fn clone_inner(&self) -> i32;
        }
    "#]]);
}

#[test]
fn a_methods_clauses_must_hold_for_the_monomorphized_type() {
    let out = run(r#"
#[derive(Debug)]
pub struct Generic<T> {
    pub value: T,
}

pub struct NotDebug;

impl<T: std::fmt::Debug> Generic<T> {
    pub fn describe(&self) -> String {
        format!("{self:?}")
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Generic<i32>, Generic<NotDebug>); 0] = [];
"#);
    // `Generic<NotDebug>` does not satisfy `T: Debug`, so the impl's methods are dropped; the
    // type itself is still emitted.
    out.assert_zng(expect![[r#"
        type crate::Generic<crate::NotDebug> {
            #layout(size = 0, align = 1);
        }

        type crate::Generic<i32> {
            #layout(size = 4, align = 4);
            wellknown_traits(Debug);

            fn describe(&self) -> ::std::string::String;
        }

        type crate::NotDebug {
            #layout(size = 0, align = 1);
        }
    "#]]);
}

#[test]
fn a_methods_own_generics_are_not_monomorphized() {
    let out = run(r#"
pub struct Generic<T> {
    pub value: T,
}

impl<T> Generic<T> {
    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn with_other<D>(&self, _other: D) -> &T {
        &self.value
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Generic<i32>,); 0] = [];
"#);
    // The instance's arguments only cover the impl's `T`; `with_other`'s own `D` cannot be
    // supplied, so it keeps its identity signature and is warn-and-skipped as an unsupported
    // type parameter. The receiver is still recognized against the ADT definition `Generic<T>`,
    // so the diagnostic names the type parameter rather than an unsupported receiver.
    out.assert_zng(expect![[r#"
        type crate::Generic<i32> {
            #layout(size = 4, align = 4);

            fn get(&self) -> &i32;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping method `with_other`: type parameters are not supported in a zngur interface
          --> src/lib.rs:13:5
           |
        13 |     pub fn with_other<D>(&self, _other: D) -> &T {
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn emit_items_accepts_a_reference_to_a_slice_of_types() {
    let out = run(r#"
pub struct Container<T> {
    pub value: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: &[(Container<i32>, Container<String>)] = &[];
"#);
    out.assert_zng(expect![[r#"
        type crate::Container<::std::string::String> {
            #layout(size = 24, align = 8);
        }

        type crate::Container<i32> {
            #layout(size = 4, align = 4);
        }
    "#]]);
}

#[test]
fn non_adt_types_are_emitted() {
    let out = run(r#"
#[cfg(autozng)]
#[autozng::emit_items]
const _: [(&str, &[u8], (u8, bool), i32, usize, ()); 0] = [];
"#);
    // `str`, slices and tuples are valid `.zng` `type` targets and are emitted (unsized types
    // carry `?Sized` instead of a layout). The empty tuple (unit) is specialised by zngur, and
    // numeric primitives (`i32`, `usize`, ...) are translated to native C++ numeric types:
    // neither is ever declared as a `type` block.
    out.assert_zng(expect![[r#"
        type (u8, bool) {
            #layout(size = 2, align = 1);
            wellknown_traits(Copy, Debug);
        }

        type [u8] {
            wellknown_traits(?Sized, Debug);
        }

        type str {
            wellknown_traits(?Sized, Debug);
        }
    "#]]);
}

#[test]
fn numeric_primitives_are_not_emitted_as_type_blocks() {
    let out = run(r#"
#[cfg(autozng)]
#[autozng::emit_items]
const _: [(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, usize, isize); 0] = [];
"#);
    // Numeric primitives are translated to native C++ numeric types; declaring them as `type`
    // blocks would generate broken C++. They are only usable in signatures, so enumerating them
    // emits nothing (and warns about nothing).
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_free_fn_named_by_an_emit_items_const_is_monomorphized() {
    let out = run(r#"
fn twice<T: Clone>(value: T) -> (T, T) {
    (value.clone(), value)
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = twice::<u8>;
};
"#);
    // A `let` binding inside a module-level `emit_items` const enumerates a free function's
    // monomorphization: its own generic args are recorded, listed as the turbofish and used to
    // instantiate the signature (the clause `u8: Clone` holds).
    out.assert_zng(expect![[r#"
        fn crate::twice<u8>(u8) -> (u8, u8);
    "#]]);
}

#[test]
fn a_trait_method_named_by_an_emit_items_const_is_attached_to_the_type() {
    let out = run(r#"
pub struct S;

impl std::fmt::Display for S {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S")
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = S::to_string;
};
"#);
    // `S::to_string` resolves to the `ToString` trait method; it is attached to `S` and keeps
    // the trait as its `use_path`.
    out.assert_zng(expect![[r#"
        type crate::S {
            #layout(size = 0, align = 1);

            fn to_string(&self) -> ::std::string::String use ::std::string::ToString;
        }
    "#]]);
}

#[test]
fn a_method_named_by_an_impl_emit_items_const_is_monomorphized() {
    let out = run(r#"
pub struct S;

impl S {
    pub fn give<D>(&self, value: D) -> D {
        value
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const MONO: () = {
        let _ = Self::give::<u8>;
    };
}
"#);
    // The `let` names a monomorphization of `S::give`: the method's own args are listed as the
    // turbofish and used to instantiate the signature. The plain (template) record of the same
    // method is skipped silently, so there is no "type parameters are not supported" warning.
    out.assert_zng(expect![[r#"
        type crate::S {
            #layout(size = 0, align = 1);

            fn give<u8>(&self, u8) -> u8;
        }
    "#]]);
}

#[test]
fn a_rename_on_a_let_binding_only_affects_that_monomorphization() {
    let out = run(r#"
pub struct S;

impl S {
    pub fn give<D>(&self, value: D) -> D {
        value
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const MONO: () = {
        let _ = Self::give::<u8>;
        #[autozng::rename("give_u32")]
        let _ = Self::give::<u32>;
    };
}
"#);
    // The `rename` on the second `let` binding is collected from the *statement's* `HirId` and
    // keyed by the named monomorphization (`give::<u32>`), so the other instantiation of the same
    // `FnDef` keeps its Rust name.
    out.assert_zng(expect![[r#"
        type crate::S {
            #layout(size = 0, align = 1);

            fn give<u8>(&self, u8) -> u8;
            fn give<u32>(&self, u32) -> u32 as give_u32;
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_case_rename_on_a_let_binding_transforms_the_method_name() {
    let out = run(r#"
pub struct S;

impl S {
    pub fn long_name<D>(&self, value: D) -> D {
        value
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const MONO: () = {
        #[autozng::rename(case = "Pascal")]
        let _ = Self::long_name::<u8>;
    };
}
"#);
    // `rename(case = ...)` transforms the named function's simple name, exactly as it would on
    // the method declaration itself.
    out.assert_zng(expect![[r#"
        type crate::S {
            #layout(size = 0, align = 1);

            fn long_name<u8>(&self, u8) -> u8 as LongName;
        }
    "#]]);
}
