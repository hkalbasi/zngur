//! The warn-and-skip policy: unsupported items must be reported and dropped without taking
//! the rest of the output with them.
//!
//! These snapshot `Run::autozng_diagnostics`, not the `.zng` output, so the expected values are
//! rustc diagnostic blocks. Paths are relative to the scratch crate and separators normalized.

use expect_test::expect;

use crate::harness::run;

#[test]
fn generic_type_is_warned_and_skipped() {
    let out = run(r#"
pub struct Generic<T> {
    pub value: T,
}
"#);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping `Generic<T>`: type parameters are not supported in a zngur interface
         --> src/lib.rs:4:1
          |
        4 | pub struct Generic<T> {
          | ^^^^^^^^^^^^^^^^^^^^^
    "#]]);
    out.assert_zng(expect![[r#""#]]);
}

#[test]
fn unsupported_field_is_warned_and_skipped() {
    let out = run(r#"
pub struct HasArray {
    pub values: [u8; 4],
    pub count: u8,
}
"#);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping field `values` of `scratch_crate::HasArray`: array types are not supported in a zngur interface
         --> src/lib.rs:5:5
          |
        5 |     pub values: [u8; 4],
          |     ^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn a_dropped_field_does_not_drop_the_type() {
    let out = run(r#"
pub struct HasArray {
    pub values: [u8; 4],
    pub count: u8,
}
"#);
    // The skipped field keeps its slot in the layout, so `count` is still at offset 4.
    out.assert_zng(expect![[r#"
        type crate::HasArray {
            #layout(size = 5, align = 1);

            field count (offset = 4, type = u8);
        }
    "#]]);
}

#[test]
fn rename_on_a_type_is_ignored() {
    let out = run(r#"
#[autozng::rename("Renamed")]
pub struct Original {
    pub value: u8,
}
"#);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: ignoring `rename` on type `scratch_crate::Original`: only associated methods can be renamed
         --> src/lib.rs:5:1
          |
        5 | pub struct Original {
          | ^^^^^^^^^^^^^^^^^^^
          |
        note: renamed here
         --> src/lib.rs:4:1
          |
        4 | #[autozng::rename("Renamed")]
          | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}
