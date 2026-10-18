//! `#[autozng::layout(...)]` — overriding a type's `.zng` layout policy.
//!
//! By default a type is emitted with `#layout(size = <real size>, align = <real align>)`. The
//! attribute records an explicit [`LayoutPolicy`](zngur_def::LayoutPolicy) on the type's metadata
//! which replaces that automatic layout during spec generation. Exactly one policy is allowed per
//! type, and an explicit `stack` / `conservative` size and alignment is checked against the type's
//! real layout.

use expect_test::expect;

use crate::harness::run;

#[test]
fn every_layout_policy_is_applied() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(stack(size = 8, align = 4)))]
pub struct Stacked {
    pub a: i32,
    pub b: i32,
}

#[cfg_attr(autozng, autozng::layout(conservative(size = 16, align = 8)))]
pub struct Wide {
    pub a: i32,
}

#[cfg_attr(autozng, autozng::layout(heap_allocated))]
pub struct Heaped {
    pub a: i32,
}

#[cfg_attr(autozng, autozng::layout(only_by_ref))]
pub struct RefOnly {
    pub a: i32,
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Heaped {
            #heap_allocate;

            field a (offset = 0, type = i32);
        }

        type crate::RefOnly {
            #only_by_ref;

            field a (offset = 0, type = i32);
        }

        type crate::Stacked {
            #layout(size = 8, align = 4);

            field a (offset = 0, type = i32);
            field b (offset = 4, type = i32);
        }

        type crate::Wide {
            #layout_conservative(size = 16, align = 8);

            field a (offset = 0, type = i32);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn stack_size_and_align_must_match_the_real_layout() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(stack(size = 4, align = 4)))]
pub struct WrongSize {
    pub a: i32,
    pub b: i32,
}

#[cfg_attr(autozng, autozng::layout(stack(size = 2, align = 4)))]
pub struct WrongAlign {
    pub a: u8,
    pub b: u8,
}
"#);
    // A `stack` override must reproduce the real layout exactly.
    out.assert_diagnostics(expect![[r#"
        error: autozng: `stack` layout size for `scratch_crate::WrongSize` must match its real size 8 (found 4)
         --> src/lib.rs:4:21
          |
        4 | #[cfg_attr(autozng, autozng::layout(stack(size = 4, align = 4)))]
          |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

        error: autozng: `stack` layout align for `scratch_crate::WrongAlign` must match its real align 1 (found 4)
          --> src/lib.rs:10:21
           |
        10 | #[cfg_attr(autozng, autozng::layout(stack(size = 2, align = 4)))]
           |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn conservative_layout_must_cover_the_real_layout() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(conservative(size = 2, align = 2)))]
pub struct TooSmall {
    pub a: i32,
}
"#);
    // A `conservative` override must be at least the real size and alignment.
    out.assert_diagnostics(expect![[r#"
        error: autozng: `conservative` layout size for `scratch_crate::TooSmall` must be at least its real size 4 (found 2)
         --> src/lib.rs:4:21
          |
        4 | #[cfg_attr(autozng, autozng::layout(conservative(size = 2, align = 2)))]
          |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

        error: autozng: `conservative` layout align for `scratch_crate::TooSmall` must be at least its real align 4 (found 2)
         --> src/lib.rs:4:21
          |
        4 | #[cfg_attr(autozng, autozng::layout(conservative(size = 2, align = 2)))]
          |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn conservative_non_power_of_two_align_is_warned_about() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(conservative(size = 4, align = 3)))]
pub struct FourBytes {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
}
"#);
    // Size 4 and alignment 3 are both at least the real (size 4, align 1), so only the
    // non-power-of-two alignment is warned about.
    out.assert_zng(expect![[r#"
        type crate::FourBytes {
            #layout_conservative(size = 4, align = 3);

            field a (offset = 0, type = u8);
            field b (offset = 1, type = u8);
            field c (offset = 2, type = u8);
            field d (offset = 3, type = u8);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#"
        warning: autozng: `conservative` layout align 3 for `scratch_crate::FourBytes` is not a power of two
         --> src/lib.rs:4:21
          |
        4 | #[cfg_attr(autozng, autozng::layout(conservative(size = 4, align = 3)))]
          |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn only_one_layout_policy_is_allowed_per_type() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(heap_allocated))]
#[cfg_attr(autozng, autozng::layout(only_by_ref))]
pub struct Two {
    pub a: i32,
}
"#);
    out.assert_diagnostics(expect![[r#"
        error: autozng: only one `autozng::layout` policy is allowed per type
         --> src/lib.rs:5:21
          |
        5 | #[cfg_attr(autozng, autozng::layout(only_by_ref))]
          |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "#]]);
}

#[test]
fn malformed_layout_arguments_are_rejected() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(unknown_policy))]
pub struct Unknown {
    pub a: i32,
}

#[cfg_attr(autozng, autozng::layout(stack(align = 4)))]
pub struct MissingSize {
    pub a: i32,
}
"#);
    // These are framework parse errors, which the `autozng:` diagnostic filter does not pick up.
    assert!(
        out.stderr.contains(
            "valid arguments are `stack`, `conservative`, `heap_allocated` or `only_by_ref`"
        ),
        "stderr:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("missing argument `size = \"...\"`"),
        "stderr:\n{}",
        out.stderr
    );
}

#[test]
fn a_layout_override_applies_to_a_concrete_generic_instance() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::layout(heap_allocated))]
#[derive(Debug)]
pub struct Generic<T> {
    pub val: T,
}

#[cfg(autozng)]
#[autozng::emit_items]
const _: &[Generic<u32>] = &[];
"#);
    // A generic ADT has no single real layout, so the override is recorded without validation and
    // applied to each concrete instantiation.
    out.assert_zng(expect![[r#"
        type crate::Generic<u32> {
            #heap_allocate;
            wellknown_traits(Debug);
        }
    "#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}
