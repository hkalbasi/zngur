//! Free functions: which ones are emitted and how unsupported ones are dropped.

use expect_test::expect;

use crate::harness::run;

#[test]
fn public_functions_are_emitted() {
    let out = run(r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn borrow(text: &str) -> usize {
    text.len()
}

pub fn into_string(value: u32) -> String {
    value.to_string()
}

fn private() {}
"#);
    out.assert_zng(expect![[r#"
        type ::std::string::String {
            #layout(size = 24, align = 8);
            wellknown_traits(Debug);
        }

        type str {
            wellknown_traits(?Sized, Debug);
        }

        safe fn crate::add(i32, i32) -> i32;
        safe fn crate::borrow(&str) -> usize;
        safe fn crate::into_string(u32) -> ::std::string::String;
    "#]]);
}

#[test]
fn a_function_returning_unit() {
    let out = run(r#"
pub fn nothing() {}
"#);
    out.assert_zng(expect![[r#"
        safe fn crate::nothing();
    "#]]);
}

#[test]
fn unsupported_signature_is_skipped() {
    let out = run(r#"
pub fn takes_array(values: [u8; 3]) -> u8 {
    values[0]
}
"#);
    out.assert_zng(expect![[r#""#]]);
}

#[test]
fn functions_are_sorted_by_path() {
    let out = run(r#"
pub fn zebra() {}
pub fn alpha() {}
pub fn in_the_middle() {}
"#);
    out.assert_zng(expect![[r#"
        safe fn crate::alpha();
        safe fn crate::in_the_middle();
        safe fn crate::zebra();
    "#]]);
}
