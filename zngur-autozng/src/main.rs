//! Small utility that print some information about a crate.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;

use std::collections::HashSet;
use std::io::stdout;
use rustc_public::run;
use rustc_public::{CompilerError, CrateDef};
use std::ops::ControlFlow;
use std::process::ExitCode;
use rustc_public::mir::{LocalDecl, MirVisitor, Terminator, TerminatorKind};
use rustc_public::mir::mono::Instance;
use rustc_public::mir::visit::Location;
use rustc_public::ty::{RigidTy, Ty, TyKind};


/// This is a wrapper that can be used to replace rustc.
fn main() -> ExitCode {
    let rustc_args: Vec<String> = std::env::args().collect();
    let result = run!(&rustc_args, start_demo);
    match result {
        Ok(_) | Err(CompilerError::Skipped | CompilerError::Interrupted(_)) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn start_demo() -> ControlFlow<()> {
    let crate_name = rustc_public::local_crate().name;
    eprintln!("--- Analyzing crate: {crate_name}");

    let crate_items = rustc_public::all_local_items();
    for item in crate_items {
        eprintln!("  - {} @{:?}", item.name(), item.span())
    }

    let entry_fn = rustc_public::entry_fn().unwrap();
    let entry_instance = Instance::try_from(entry_fn).unwrap();
    analyze_instance(entry_instance);
    ControlFlow::Break(())
}

fn analyze_instance(instance: Instance) {
    eprintln!("--- Analyzing instance: {}", instance.name());
    eprintln!("  - Mangled name: {}", instance.mangled_name());
    eprintln!("  - FnABI: {:?}", instance.fn_abi().unwrap());

    let body = instance.body().unwrap();
    let mut visitor = Visitor {
        locals: body.locals(),
        tys: Default::default(),
        fn_calls: Default::default(),
    };
    visitor.visit_body(&body);
    visitor.tys.iter().for_each(|ty| eprintln!("  - Visited: {ty}"));
    visitor.fn_calls.iter().for_each(|instance| eprintln!("  - Call: {}", instance.name()));

    body.dump(&mut stdout().lock(), &instance.name()).unwrap();
}

struct Visitor<'a> {
    locals: &'a [LocalDecl],
    tys: HashSet<Ty>,
    fn_calls: HashSet<Instance>,
}

impl<'a> MirVisitor for Visitor<'a> {
    fn visit_terminator(&mut self, term: &Terminator, _location: Location) {
        match term.kind {
            TerminatorKind::Call { ref func, .. } => {
                let op_ty = func.ty(self.locals).unwrap();
                let TyKind::RigidTy(RigidTy::FnDef(def, args)) = op_ty.kind() else { return; };
                self.fn_calls.insert(Instance::resolve(def, &args).unwrap());
            }
            _ => {}
        }
    }

    fn visit_ty(&mut self, ty: &Ty, _location: Location) {
        self.tys.insert(*ty);
    }
}

// use std::collections::HashMap;
//
// use serde::{Deserialize, Serialize};
// use serde_json::Value;
//
// #[derive(Debug, Serialize, Deserialize)]
// #[serde(rename_all = "snake_case")]
// enum RustdocRustType {
//     BorrowedRef {
//         mutable: bool,
//         #[serde(rename = "type")]
//         inner: Box<RustdocRustType>,
//     },
//     RawPointer {
//         mutable: bool,
//         #[serde(rename = "type")]
//         inner: Box<RustdocRustType>,
//     },
//     Primitive(String),
//     Generic(String),
//     Tuple(Vec<RustdocRustType>),
//     Slice(Box<RustdocRustType>),
//     ResolvedPath {
//         name: String,
//     },
//     QualifiedPath {},
// }
//
// impl RustdocRustType {
//     fn render(&self) -> String {
//         match self {
//             RustdocRustType::BorrowedRef {
//                 mutable: false,
//                 inner,
//             } => format!("&{}", inner.render()),
//             RustdocRustType::BorrowedRef {
//                 mutable: true,
//                 inner,
//             } => format!("&mut {}", inner.render()),
//             RustdocRustType::RawPointer { .. } => todo!(),
//             RustdocRustType::Primitive(n) => n.clone(),
//             RustdocRustType::Generic(n) => n.clone(),
//             RustdocRustType::Tuple(_) => todo!(),
//             RustdocRustType::Slice(_) => todo!(),
//             RustdocRustType::ResolvedPath { name } => name.clone(),
//             RustdocRustType::QualifiedPath {} => todo!(),
//         }
//     }
// }
//
// #[derive(Debug, Serialize, Deserialize)]
// struct RustdocFunctionDecl {
//     inputs: Vec<(String, RustdocRustType)>,
//     output: Option<RustdocRustType>,
//     #[serde(flatten)]
//     other_fields: Value,
// }
//
// #[derive(Debug, Serialize, Deserialize)]
// #[serde(rename_all = "snake_case")]
// enum RustdocItemInner {
//     Function {
//         decl: RustdocFunctionDecl,
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Struct {
//         impls: Vec<String>,
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Impl {
//         items: Vec<String>,
//         #[serde(rename = "trait")]
//         for_trait: Option<serde_json::Value>,
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Module {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     StructField {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Import {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     AssocType {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Variant {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     TypeAlias {
//         #[serde(flatten)]
//         other_fields: Value,
//     },
//     Enum {
//         impls: Vec<String>,
//         #[serde(flatten)]
//         other_fields: Value,
//     },
// }
//
// #[derive(Debug, Serialize, Deserialize)]
// struct RustdocItem {
//     name: Option<String>,
//     inner: RustdocItemInner,
//     #[serde(flatten)]
//     other_fields: Value,
// }
//
// #[derive(Serialize, Deserialize)]
// struct RustdocOutput {
//     index: HashMap<String, RustdocItem>,
// }
//
// fn main() {
//     let s = std::fs::read_to_string("./doc.json").unwrap();
//     let d: RustdocOutput = serde_json::from_str(&s).unwrap();
//     for x in &d.index {
//         if let RustdocItemInner::Struct { impls, .. } | RustdocItemInner::Enum { impls, .. } =
//             &x.1.inner
//         {
//             println!("type crate::{} {{", x.1.name.as_ref().unwrap());
//             println!("    #heap_allocated;");
//             for imp in impls {
//                 let imp = &d.index[imp];
//                 // dbg!(imp);
//
//                 if let RustdocItemInner::Impl {
//                     items, for_trait, ..
//                 } = &imp.inner
//                 {
//                     if for_trait.is_some() {
//                         continue;
//                     }
//                     for item in items {
//                         let item = &d.index[item];
//                         if let RustdocItemInner::Function {
//                             decl: RustdocFunctionDecl { inputs, output, .. },
//                             ..
//                         } = &item.inner
//                         {
//                             print!("    fn {}(", item.name.as_deref().unwrap());
//                             let mut first = true;
//                             for (name, ty) in inputs {
//                                 if !first {
//                                     print!(", ");
//                                 }
//                                 first = false;
//                                 if name == "self" {
//                                     print!("self");
//                                     continue;
//                                 }
//                                 print!("{}", ty.render());
//                             }
//                             print!(")");
//                             if let Some(output) = output {
//                                 print!(" -> {}", output.render());
//                             }
//                             println!(";");
//                         }
//                     }
//                 }
//             }
//             println!("}}");
//         }
//     }
// }
