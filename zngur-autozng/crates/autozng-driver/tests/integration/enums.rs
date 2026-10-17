//! Enums: variants, their fields and non-exhaustiveness.

use expect_test::expect;

use crate::harness::run;

#[test]
fn unit_tuple_and_struct_variants() {
    let out = run(r#"
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Nothing,
    Circle(u32),
    Rect { width: u32, height: u32 },
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Shape {
            #layout(size = 12, align = 4);
            wellknown_traits(Copy, Debug);
            variant Nothing {}
            variant Circle {
                field 0 (offset = auto, type = u32);
            }
            variant Rect {
                field width (offset = auto, type = u32);
                field height (offset = auto, type = u32);
            }
        }
    "#]]);
}

#[test]
fn fields_of_variants_follow_field_selection() {
    let out = run(r#"
pub enum Payload {
    Text(String),
    Number { value: i32, extra: i32 },
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Payload {
            #layout(size = 24, align = 8);
            variant Text {
                field 0 (offset = auto, type = ::std::string::String);
            }
            variant Number {
                field value (offset = auto, type = i32);
                field extra (offset = auto, type = i32);
            }
        }
    "#]]);
}

#[test]
fn non_exhaustive_enum() {
    let out = run(r#"
#[non_exhaustive]
pub enum Kind {
    One,
    Two,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Kind {
            #layout(size = 1, align = 1);
            non_exhaustive;
            variant One {}
            variant Two {}
        }
    "#]]);
}
