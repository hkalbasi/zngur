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

// fn analyze_instance(instance: Instance) {
//     eprintln!("--- Analyzing instance: {}", instance.name());
//     eprintln!("  - Mangled name: {}", instance.mangled_name());
//     eprintln!("  - FnABI: {:?}", instance.fn_abi().unwrap());

//     let body = instance.body().unwrap();
//     let mut visitor = Visitor {
//         locals: body.locals(),
//         tys: Default::default(),
//         fn_calls: Default::default(),
//     };
//     visitor.visit_body(&body);
//     visitor
//         .tys
//         .iter()
//         .for_each(|ty| eprintln!("  - Visited: {ty}"));
//     visitor
//         .fn_calls
//         .iter()
//         .for_each(|instance| eprintln!("  - Call: {}", instance.name()));

//     body.dump(&mut stdout().lock(), &instance.name()).unwrap();
// }

// struct Visitor<'a> {
//     locals: &'a [LocalDecl],
//     tys: HashSet<Ty>,
//     fn_calls: HashSet<Instance>,
// }

// impl<'a> MirVisitor for Visitor<'a> {
//     fn visit_terminator(&mut self, term: &Terminator, _location: Location) {
//         match term.kind {
//             TerminatorKind::Call { ref func, .. } => {
//                 let op_ty = func.ty(self.locals).unwrap();
//                 let TyKind::RigidTy(RigidTy::FnDef(def, args)) = op_ty.kind() else {
//                     return;
//                 };
//                 self.fn_calls.insert(Instance::resolve(def, &args).unwrap());
//             }
//             _ => {}
//         }
//     }

//     fn visit_ty(&mut self, ty: &Ty, _location: Location) {
//         self.tys.insert(*ty);
//     }
// }
