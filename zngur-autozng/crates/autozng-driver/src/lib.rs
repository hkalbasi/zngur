//! `autozng`: a `rustc` compiler-driver that generates Zngur `.zng` interface files from the
//! public API of a Rust crate.
//!
//! Start with the `analyze` and `render` modules for the pipeline, and `attrs` for the
//! attribute semantics.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_ast_lowering;
extern crate rustc_ast_pretty;
extern crate rustc_attr_ir;
extern crate rustc_attr_parsing;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_hir_analysis;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_macros;
extern crate rustc_middle;
extern crate rustc_parse;
extern crate rustc_proc_macro;
extern crate rustc_public;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_trait_selection;
extern crate rustc_type_ir;

extern crate rustc_hir_pretty;

pub mod run;

mod analyze;
mod attrs;
mod ctx;
mod error;
mod metadata;
mod render;
mod rustc_ext;
mod span_tools;
mod visit;

mod type_map;

mod config;

mod driver;

mod version;

mod walk;
