//! The warn-and-skip policy: unsupported items must be reported and dropped without taking
//! the rest of the output with them.
//!
//! These snapshot `Run::autozng_diagnostics`, not the `.zng` output, so the expected values are
//! rustc diagnostic blocks. Paths are relative to the scratch crate and separators normalized.

use expect_test::expect;

use crate::harness::run;

#[test]
fn generic_type_is_skipped_silently() {
    let out = run(r#"
pub struct Generic<T> {
    pub value: T,
}
"#);
    // A generic template has no `.zng` representation of its own; only its concrete
    // instantiations are emitted (via `emit_items`). A type parameter is expected here, so
    // the template is dropped without a diagnostic.
    out.assert_diagnostics(expect![[r#""#]]);
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
fn unresolved_projection_is_skipped_silently() {
    let out = run(r#"
pub trait HasAssoc {
    type Output;
}

impl HasAssoc for u32 {
    type Output = i64;
}

struct Holder<T> {
    v: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: [(Holder<<u32 as HasAssoc>::Output>,); 0] = [];
"#);
    // The instantiation still mentions an unresolved associated-type projection, so it is
    // dropped without any diagnostics (unlike a bare type parameter, which is warned about).
    out.assert_diagnostics(expect![[r#""#]]);
    out.assert_zng(expect![[r#""#]]);
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

#[test]
fn an_enumerated_non_adt_type_warns_at_the_const_span() {
    let out = run(r#"
#[cfg(autozng)]
#[autozng::emit_items]
const _: [(&[u8; 4],); 0] = [];
"#);
    // A non-ADT type (an array here) has no `DefId` of its own, so the `const` tagged by
    // `#[autozng::emit_items]` becomes the primary span of the warning.
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping `[u8; 4]`: array types are not supported in a zngur interface
         --> src/lib.rs:6:1
          |
        6 | const _: [(&[u8; 4],); 0] = [];
          | ^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn an_item_selected_by_a_let_binding_notes_the_emit_site() {
    let out = run(r#"
pub struct S;

impl S {
    pub fn give<D: Copy>(&self, value: D) -> [D; 4] {
        [value; 4]
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const MONO: () = {
        let _ = Self::give::<u8>;
    };
}
"#);
    // `give::<u8>` is instantiated and emitted, but its `[D; 4]` return type is unsupported: the
    // primary span is the method, and the `let` that selected it is noted as the emission site.
    out.assert_diagnostics(expect![[r#"
        warning: autozng: skipping method `give`: array types are not supported in a zngur interface
          --> src/lib.rs:7:5
           |
         7 |     pub fn give<D: Copy>(&self, value: D) -> [D; 4] {
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
           |
        note: emitted here
          --> src/lib.rs:14:9
           |
        14 |         let _ = Self::give::<u8>;
           |         ^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn a_rename_on_a_free_fn_let_binding_is_ignored() {
    let out = run(r#"
fn twice<T: Clone>(value: T) -> (T, T) {
    (value.clone(), value)
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename("Twice")]
    let _ = twice::<u8>;
};
"#);
    // The `rename` on the `let` binding *is* collected (it is not silently dropped), but a free
    // function has no `cpp_name` slot in the IDL, so it is ignored with a warning whose note
    // points at the binding's attribute.
    out.assert_diagnostics(expect![[r#"
        warning: autozng: ignoring `rename` on fn `scratch_crate::twice`: only associated methods can be renamed
          --> src/lib.rs:4:1
           |
         4 | fn twice<T: Clone>(value: T) -> (T, T) {
           | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
           |
        note: renamed here
          --> src/lib.rs:11:5
           |
        11 |     #[autozng::rename("Twice")]
           |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}
