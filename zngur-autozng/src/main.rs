//! Small utility that print some information about a crate.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;
extern crate rustc_span;

use rustc_middle::ty::TyCtxt;
use rustc_public::abi::VariantsShape;
use rustc_public::run_with_tcx;
use rustc_public::target::MachineSize;
use rustc_public::ty::AdtKind;
use rustc_public::{CompilerError, CrateDef};

use crate::rustc_public::CrateDefType;

use std::ops::ControlFlow;
use std::process::ExitCode;
mod ctx;

use ctx::AutoZngContext;

/// This is a wrapper that can be used to replace rustc.
fn main() -> ExitCode {
    let rustc_args: Vec<String> = std::env::args().collect();
    let result = run_with_tcx!(&rustc_args, analyze);
    match result {
        Ok(_) | Err(CompilerError::Skipped | CompilerError::Interrupted(_)) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn analyze(tcx: TyCtxt) -> ControlFlow<()> {
    let ctx = AutoZngContext::new(tcx);

    let crate_name = rustc_public::local_crate().name;
    eprintln!("--- Analyzing crate: {crate_name}");

    let krate = rustc_public::local_crate();
    let adts = krate.adts();
    for item in adts {
        let layout = item.ty().layout().unwrap().shape();
        let docs = ctx.docs_for(item.def_id());
        eprintln!(
            "  - {} @{:?} @Layout{{ size: {}, align: {} }}",
            item.trimmed_name(),
            item.span(),
            layout.size.bytes(),
            layout.abi_align
        );
        for attr in item.all_tool_attrs() {
            eprintln!("    - TOOL_ATTR: {}", attr.as_str());
        }
        for (doc, _span) in docs {
            eprintln!("    - Docs: {}", doc);
        }
        eprintln!(
            "    - {} ({} variants) {{",
            match item.kind() {
                AdtKind::Enum => "Enum",
                AdtKind::Union => "Union",
                AdtKind::Struct => "Struct",
            },
            item.num_variants(),
        );
        eprintln!("      FIELDS: {:?}", &layout.fields);
        for (_v, variant) in item.variants().iter().enumerate() {
            eprintln!("      {} {{", variant.name());
            eprintln!("      FIELDS: {:?}", &variant.fields());
            for (f, field) in variant.fields().iter().enumerate() {
                let offset =
                    if let rustc_public::abi::FieldsShape::Arbitrary { offsets } = &layout.fields {
                        offsets.get(f).copied()
                    } else {
                        None
                    };
                let is_pub = ctx.is_public_vis(field.def_id());
                let field_name = field
                    .trimmed_name()
                    .trim_start_matches(&(item.trimmed_name() + "::"))
                    .trim_start_matches(&(variant.name() + "::"))
                    .to_string();
                eprintln!(
                    "        {}{}: {}, @offset({})",
                    if is_pub { "pub " } else { "" },
                    field_name,
                    field.ty(),
                    offset.map(|o| o.bytes()).unwrap_or_default(),
                )
            }
            eprintln!("      }},");
        }
        eprintln!("    }}");
    }
    for impl_ in krate.trait_impls() {
        eprintln!("  Impl: {}", impl_.name());
        for item in impl_.associated_items() {
            eprintln!("    - assoc: {}", item.def_id.name());
        }
    }

    let crate_items = rustc_public::all_local_items();
    for item in crate_items {
        let docs = ctx.docs_for(item.def_id());
        if let Some(owner_docs) = ctx.docs_for_owner(item.def_id()) {
            eprintln!("  Owner Docs: ",);
            for (line, _span) in owner_docs {
                eprintln!("    - Docs: {line} ");
            }
        }
        let is_pub = ctx.is_public_vis(item.def_id());
        eprintln!(
            "  - {}fn {} @{:?}",
            if is_pub { "pub " } else { "" },
            item.name(),
            item.span()
        );
        for (doc, _span) in docs {
            eprintln!("   - Docs: {}", doc);
        }
        eprintln!(
            "    - Kind: {} ",
            match item.kind() {
                rustc_public::ItemKind::Fn => "Fn",
                rustc_public::ItemKind::Static => "Static",
                rustc_public::ItemKind::Const => "Const",
                rustc_public::ItemKind::Ctor(kind) => match kind {
                    rustc_public::CtorKind::Const => "Ctor(Const)",
                    rustc_public::CtorKind::Fn => "Ctor(Fn)",
                },
            }
        );
        if let Some(fn_sig) = item.ty().kind().fn_sig() {
            let output = fn_sig.value.output();
            let inputs = fn_sig.value.inputs();
            let inputs_str = inputs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("    - @Sig ({}) -> {} ", inputs_str, output);
        }
    }

    // let entry_fn = rustc_public::entry_fn().unwrap();
    // let entry_instance = Instance::try_from(entry_fn).unwrap();
    // analyze_instance(entry_instance);
    ControlFlow::Break(())
}

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
