//! Re-exported *external* items (`pub use std::num::*;`).
//!
//! A crate can make another crate's items its public API with a `pub use`. The crate walk only
//! visits the local crate, so those items are recorded by the re-export pass
//! (`analyze::visit_reexported_items`) instead — without it a crate whose whole API is re-exported
//! emits an empty `.zng`. The rendered path is `crate::...`: the generated code lives inside the
//! crate and names the item through the re-export, not through its definition path.

use expect_test::expect;

use crate::harness::run;

#[test]
fn a_named_reexport_of_an_external_type_is_emitted_under_the_crate_path() {
    let run = run("pub use std::num::ParseIntError;\n");
    run.assert_zng(expect![[r#"
        type crate::ParseIntError {
            #layout(size = 1, align = 1);
            wellknown_traits(Debug);
        }
    "#]]);
    run.assert_diagnostics(expect![[""]]);
}

#[test]
fn a_glob_reexport_of_an_external_module_emits_its_public_items() {
    let run = run("pub use std::num::*;\n");
    run.assert_zng(expect![[r#"
        type crate::FpCategory {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);
            variant Nan {}
            variant Infinite {}
            variant Zero {}
            variant Subnormal {}
            variant Normal {}
        }

        type crate::IntErrorKind {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);
            non_exhaustive;
            variant Empty {}
            variant InvalidDigit {}
            variant PosOverflow {}
            variant NegOverflow {}
            variant Zero {}
            variant NotAPowerOfTwo {}
        }

        type crate::ParseFloatError {
            #layout(size = 1, align = 1);
            wellknown_traits(Debug);
        }

        type crate::ParseIntError {
            #layout(size = 1, align = 1);
            wellknown_traits(Debug);
        }

        type crate::TryFromIntError {
            #layout(size = 1, align = 1);
            wellknown_traits(Copy, Debug);
        }

        trait crate::ZeroablePrimitive {}
    "#]]);
    run.assert_diagnostics(expect![[""]]);
}
