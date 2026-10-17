//! Rendering of the analyzed crate to Zngur `.zng` IDL output.
//!
//! This module is currently empty. The intended design (confirmed with the architect) is a
//! two-stage split driven by a lifetime constraint:
//!
//! ```text
//! (inside after_analysis, `TyCtxt` alive)          (any time after)
//! AutozngContext  ->  zngur_def::ZngurSpec   ->    IDLPrinter::print  ->  .zng text
//!                         build_spec()                 print()
//! ```
//!
//! The `rustc`/`rustc_public` types held by [`AutozngContext`](crate::ctx::AutozngContext)
//! borrow from the rustc `TyCtxt`, which only exists for the duration of the `after_analysis`
//! callback. Therefore **the `ZngurSpec` must be built from the context while the `TyCtxt` is
//! still in scope**. The spec itself is owned and lifetime-free, so printing it may happen
//! later (e.g. after `analyze` has returned).
//!
//! Building the [`zngur_def::ZngurSpec`] is the hard part: it needs methods, constructors,
//! fields, variants, layout policy and well-known traits, none of which the analysis pass
//! records yet (see `AutozngContext::record_impl_method`). `zngur-def` already contains an
//! `IDLPrinter` (behind the `printing` feature, which this crate enables) that serializes a
//! `ZngurSpec` back to `.zng` syntax.
//!
//! See the `analyze`, `ctx` and `type_map` modules for the data this consumes.
