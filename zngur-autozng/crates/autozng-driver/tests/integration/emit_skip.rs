//! The `emit` / `skip` attributes on types, fields and methods.

use expect_test::expect;

use crate::harness::run;

#[test]
fn emit_a_private_type() {
    let out = run(r#"
#[autozng::emit]
struct Hidden {
    pub value: u8,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Hidden {
            #layout(size = 1, align = 1);

            field value (offset = 0, type = u8);
        }
    "#]]);
}

#[test]
fn skip_a_public_type() {
    let out = run(r#"
#[autozng::skip]
pub struct Skipped;

pub struct Kept;
"#);
    out.assert_zng(expect![[r#"
        type crate::Kept {
            #layout(size = 0, align = 1);
        }
    "#]]);
}

#[test]
fn emit_named_private_fields() {
    let out = run(r#"
#[autozng::emit(second, third)]
pub struct Selection {
    pub first: u8,
    second: u16,
    third: u32,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Selection {
            #layout(size = 8, align = 4);

            field first (offset = 6, type = u8);
            field second (offset = 4, type = u16);
            field third (offset = 0, type = u32);
        }
    "#]]);
}

#[test]
fn emit_a_field_directly() {
    let out = run(r#"
pub struct Direct {
    pub kept: u8,
    #[autozng::emit]
    forced: u16,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Direct {
            #layout(size = 4, align = 2);

            field kept (offset = 2, type = u8);
            field forced (offset = 0, type = u16);
        }
    "#]]);
}

#[test]
fn skip_named_public_fields() {
    let out = run(r#"
#[autozng::skip(dropped)]
pub struct Fields {
    pub kept: u8,
    pub dropped: u8,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Fields {
            #layout(size = 2, align = 1);

            field kept (offset = 0, type = u8);
        }
    "#]]);
}

#[test]
fn emit_a_private_method() {
    let out = run(r#"
pub struct Api;

impl Api {
    pub fn visible(&self) {}

    #[autozng::emit]
    fn hidden(&self) {}
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Api {
            #layout(size = 0, align = 1);

            safe fn visible(&self);
            safe fn hidden(&self);
        }
    "#]]);
}

#[test]
fn skip_a_public_method() {
    let out = run(r#"
pub struct Api;

impl Api {
    pub fn kept(&self) {}

    #[autozng::skip]
    pub fn dropped(&self) {}
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Api {
            #layout(size = 0, align = 1);

            safe fn kept(&self);
        }
    "#]]);
}
