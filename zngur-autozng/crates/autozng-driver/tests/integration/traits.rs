//! Trait declarations: an `emit`-tagged `pub`/`pub(crate)` trait and its associated functions
//! are recorded into the analysis context.
//!
//! Trait *rendering* is not implemented yet, so an emitted trait produces no `.zng`; these tests
//! assert that recording runs (without leaking the trait or its methods into the type/function
//! output, and without diagnostics). When `render` learns to emit traits the `.zng` snapshots
//! below will need regenerating.

use expect_test::expect;

use crate::harness::run;

#[test]
fn an_emit_tagged_trait_is_recorded_without_output_yet() {
    let out = run(r#"
#[cfg_attr(autozng, autozng::emit)]
pub trait Greeter<T: std::fmt::Debug> {
    fn greet(&self) -> T;
    fn set(&mut self, value: T);
    fn generic<D>(&self, value: D) -> D;
}
"#);
    // The trait and its methods are recorded into `AutozngContext`; nothing is rendered yet, and
    // in particular the methods must not leak in as free functions or type methods.
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}

#[test]
fn a_trait_without_emit_is_not_recorded() {
    let out = run(r#"
pub trait NotEmitted {
    fn ignored(&self);
}
"#);
    // A trait's implicit emit action is `None`: only an explicit `emit` selects it.
    out.assert_zng(expect![[r#""#]]);
    out.assert_diagnostics(expect![[r#""#]]);
}
