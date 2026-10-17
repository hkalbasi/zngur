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
fn public_items_inside_private_modules_are_emitted() {
    let out = run(r#"
mod inner {
    pub struct Inner;
}
"#);
    out.assert_zng(expect![[r#"
        type crate::inner::Inner {
            #layout(size = 0, align = 1);
        }
    "#]]);
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
