//! Structs and unions: layout, field selection and field offsets.

use expect_test::expect;

use crate::harness::run;

#[test]
fn public_fields_with_offsets() {
    let out = run(r#"
pub struct Point {
    pub x: i32,
    pub y: i32,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Point {
            #layout(size = 8, align = 4);

            field x (offset = 0, type = i32);
            field y (offset = 4, type = i32);
        }
    "#]]);
}

#[test]
fn wellknown_traits() {
    let out = run(r#"
#[derive(Clone, Copy, Debug, Default)]
pub struct Flag {
    pub on: bool,
}
"#);
    out.assert_zng(expect![[r#"
        type bool {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);
        }

        type crate::Flag {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);

            field on (offset = 0, type = bool);
        }
    "#]]);
}

#[test]
fn private_fields_are_not_emitted() {
    let out = run(r#"
pub struct Mixed {
    pub visible: u8,
    hidden: u8,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Mixed {
            #layout(size = 2, align = 1);

            field visible (offset = 0, type = u8);
        }
    "#]]);
}

#[test]
fn fields_follow_rustcs_reordering() {
    // rustc reorders fields to minimize padding; the rendered offsets must follow it, even
    // though the fields themselves are listed in declaration order.
    let out = run(r#"
pub struct Reordered {
    pub small: u8,
    pub large: u32,
    pub medium: u16,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Reordered {
            #layout(size = 8, align = 4);

            field small (offset = 6, type = u8);
            field large (offset = 0, type = u32);
            field medium (offset = 4, type = u16);
        }
    "#]]);
}

#[test]
fn union_fields_have_no_offsets() {
    let out = run(r#"
pub union Bits {
    pub integer: u32,
    pub float: f32,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Bits {
            #layout(size = 4, align = 4);

            field integer (offset = auto, type = u32);
            field float (offset = auto, type = f32);
        }
    "#]]);
}
