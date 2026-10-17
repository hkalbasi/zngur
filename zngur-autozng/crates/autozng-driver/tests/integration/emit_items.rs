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
