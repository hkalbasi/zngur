use cpp::CppExportedFnDefinition;
use cpp::CppExportedImplDefinition;
use cpp::CppFile;
use cpp::CppFnDefinition;
use cpp::CppFnSig;
use cpp::CppMethod;
use cpp::CppPath;
use cpp::CppTraitDefinition;
use cpp::CppTypeDefinition;
use cpp::CppVariant;
use indexmap::map::Entry;
use itertools::Itertools;
use rust::CoroSupport;
use rust::IntoCpp;

pub mod cpp;
mod rust;
mod template;

use askama::Template;
pub use rust::RustFile;
pub use zngur_parser::{
    ParseReport, ParseResult, ParsedZngFile, ReportEntry, ReportSink, ReportSpan, SourceCache,
    SourceId, StdErrReportSink, cfg,
};

pub use zngur_def::*;

use crate::template::ZngHeaderTemplate;

#[derive(Default)]
struct CppModuleTree {
    children: indexmap::IndexMap<String, CppModuleTree>,
    content: String,
}

impl CppModuleTree {
    fn insert(&mut self, path: &[String], content: &str) {
        match path.split_first() {
            None => self.content.push_str(content),
            Some((head, rest)) => self
                .children
                .entry(head.clone())
                .or_default()
                .insert(rest, content),
        }
    }

    fn render(&self) -> String {
        let mut out = self.content.clone();
        for (name, child) in &self.children {
            out.push_str(&format!("\npub mod {name} {{\n{}\n}}\n", child.render()));
        }
        out
    }
}

pub struct ZngurGenerator(pub ZngurSpec, pub String);

impl ZngurGenerator {
    pub fn build_from_zng(zng: ZngurSpec, crate_name: String) -> Self {
        ZngurGenerator(zng, crate_name)
    }

    pub fn render(self, zng_header_in_place: bool) -> (String, String, Option<String>) {
        let zng = self.0;
        let mut cpp_file = CppFile::default();
        cpp_file.header_file_name = zng.cpp_include_header_name.clone();
        cpp_file.additional_includes = zng.additional_includes.0;
        cpp_file.zng_header_in_place = zng_header_in_place;
        let mut included_headers = std::collections::HashSet::new();
        for module in &zng.imported_modules {
            if included_headers.insert(&module.path) {
                cpp_file
                    .additional_includes
                    .push_str(&format!("\n#include \"{}.h\"", module.path.display()));
            }
        }
        let default_ns = zng.cpp_namespace.as_deref().unwrap_or("rust");
        let sanitized_crate_name = self.1.replace('-', "_");
        let mut rust_file = RustFile::new(&zng.mangling_base);
        rust_file.panic_to_exception = zng.convert_panic_to_exception.0;
        let coro_support = CoroSupport::from_types(
            zng.types.iter().map(|td| &td.ty),
            &zng.mangling_base,
            default_ns,
            &sanitized_crate_name,
        );
        if let Some(coro) = &coro_support {
            rust_file.add_coro_support(coro);
        }
        cpp_file.trait_defs = zng
            .traits
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    rust_file.add_builder_for_dyn_trait(value, default_ns, &sanitized_crate_name),
                )
            })
            .collect();
        cpp_file.panic_to_exception = zng.convert_panic_to_exception.0;
        cpp_file
            .rust_cfg_defines
            .extend(zng.rust_cfg.iter().map(|(key, value)| {
                format!(
                    "ZNGUR_CFG_{}{}",
                    key.to_uppercase(),
                    value
                        .as_ref()
                        .and_then(|value| if value.trim().is_empty() {
                            None
                        } else {
                            Some(format!(
                                "_{}",
                                value
                                    .chars()
                                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                                    .collect::<String>()
                                    .to_uppercase()
                            ))
                        })
                        .unwrap_or_default()
                )
            }));
        let mut cpp_tree = CppModuleTree::default();
        cpp_file.coro_support = coro_support;
        for ty_def in zng.types {
            let ty = &ty_def.ty;
            // What Rust can do with a C++ (`c++::`) type: hold it by value if it has a layout and
            // is trivially relocatable, own a heap-allocated C++ pointer if `#heap_allocated`,
            // and otherwise only refer to it.
            let cpp_name = ty_def.cpp_name();
            let cpp_by_value_type = if ty_def.cpp_by_value_layout().is_some() {
                cpp_name.clone()
            } else {
                None
            };
            let is_cpp_heap_allocated = ty_def.is_cpp_heap_allocated();
            let cpp_ref = if cpp_by_value_type.is_none() && !is_cpp_heap_allocated {
                cpp_name.clone().map(|name| cpp::CppRef {
                    name,
                    layout: ty_def.layout,
                })
            } else {
                None
            };
            let is_copy = ty_def.wellknown_traits.contains(&ZngurWellknownTrait::Copy);
            // In Rust, C++ types that are only referred to are zero-sized. (Their layout, if
            // any, is only checked in C++.)
            let layout = if cpp_ref.is_some() {
                LayoutPolicy::ZERO_SIZED_TYPE
            } else {
                ty_def
                    .layout
                    .expect("Every type should have a defined layout policy before rendering")
            };
            match layout {
                LayoutPolicy::StackAllocated { size, align } => {
                    rust_file.add_static_size_assert(&ty, size);
                    rust_file.add_static_align_assert(&ty, align);
                }
                LayoutPolicy::Conservative { size, align } => {
                    rust_file.add_static_size_upper_bound_assert(&ty, size);
                    rust_file.add_static_align_upper_bound_assert(&ty, align);
                }
                LayoutPolicy::HeapAllocated => (),
                LayoutPolicy::OnlyByRef => (),
            }
            if is_copy {
                rust_file.add_static_is_copy_assert(&ty);
            }
            let mut cpp_stack_owned_data = None;
            if let RustType::Cpp(segs) = ty {
                let (type_name, module_path) = segs
                    .split_last()
                    .expect("Cpp path must have at least one segment");
                if let Some(cpp_type) = cpp_by_value_type {
                    let mangled_name = rust_file.cpp_stack_owned_destructor_fn(ty);
                    cpp_stack_owned_data = Some(CppStackOwnedData {
                        destructor_fn: mangled_name.clone(),
                        cpp_type,
                    });
                    let LayoutPolicy::StackAllocated { size, align } = layout else {
                        unreachable!("C++ types held by value have a size and alignment");
                    };
                    let struct_def = format!(
                        r#"
    #[repr(C)]
    #[repr(align({align}))]
    pub struct {type_name} {{
      pub(crate) buffer: ::core::cell::UnsafeCell<::core::mem::MaybeUninit<[u8; {size}]>>,
      _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
      _pinned: ::core::marker::PhantomPinned,
    }}

    unsafe impl ::zngur_lib::ZngCppObject for {type_name} {{}}
    unsafe impl ::zngur_lib::ZngCppStackObject for {type_name} {{}}

    unsafe impl ::zngur_lib::ZngCppDestruct for {type_name} {{
        unsafe fn destruct(&mut self) {{
            unsafe extern "C" {{
                fn {mangled_name}(i0: *mut u8);
            }}
            unsafe {{
                {mangled_name}(self.buffer.get() as *mut _);
            }}
        }}
    }}

    impl Drop for {type_name} {{
        fn drop(&mut self) {{
            use ::zngur_lib::ZngCppDestruct;
            unsafe {{
                self.destruct();
            }}
        }}
    }}
"#
                    );
                    cpp_tree.insert(module_path, &struct_def);
                }
                if is_cpp_heap_allocated {
                    let struct_def = format!(
                        r#"
    #[repr(C)]
    pub struct {type_name} {{
        pub(crate) data: *mut u8,
        pub(crate) destructor: extern "C" fn(*mut u8),
    }}

    impl Drop for {type_name} {{
        fn drop(&mut self) {{
            (self.destructor)(self.data)
        }}
    }}
"#
                    );
                    cpp_tree.insert(module_path, &struct_def);
                }
                if cpp_ref.is_some() {
                    let struct_def = format!(
                        r#"
    pub struct {type_name}(());
"#
                    );
                    cpp_tree.insert(module_path, &struct_def);
                }
            }
            let mut cpp_methods = vec![];
            let mut constructor = None;
            let mut variants = vec![];
            let mut fields = vec![];
            let mut wellknown_traits = vec![];
            if let Some(constructor_def) = ty_def.constructor {
                let rust_link_name = rust_file.add_constructor(
                    &ty.to_string(),
                    constructor_def.inputs.iter().map(|(name, ty)| (name, ty)),
                );
                constructor = Some(CppFnSig {
                    rust_link_name,
                    inputs: constructor_def
                        .inputs
                        .iter()
                        .map(|x| x.1.into_cpp(default_ns, &sanitized_crate_name))
                        .collect(),
                    output: ty.into_cpp(default_ns, &sanitized_crate_name),
                });
            }
            let discriminant = if ty_def.exhaustive {
                rust_file.add_discriminant(&ty.to_string(), &ty_def.variants)
            } else {
                None
            };
            for variant in ty_def.variants {
                let rust_name = format!("{}::{}", ty, variant.name);
                let constructor = if variant.exhaustive {
                    let rust_link_name = rust_file.add_constructor(
                        &rust_name,
                        variant.fields.iter().map(|field| (&field.name, &field.ty)),
                    );
                    Some(CppFnSig {
                        rust_link_name,
                        inputs: variant
                            .fields
                            .iter()
                            .map(|x| x.ty.into_cpp(default_ns, &sanitized_crate_name))
                            .collect(),
                        output: ty
                            .into_cpp(default_ns, &sanitized_crate_name)
                            .with_tail(cpp_handle_keyword(&variant.name)),
                    })
                } else {
                    None
                };
                let match_check = rust_file.add_match_check(&rust_name);
                let mut fields = vec![];
                for field in variant.fields {
                    let mn =
                        rust_file.add_variant_field_calculations(&field, &ty_def.ty, &variant.name);
                    fields.push(ZngurFieldData {
                        name: field.name,
                        ty: field.ty,
                        offset: ZngurFieldDataOffset::AutoDynamic(mn),
                    });
                }
                variants.push(CppVariant {
                    name: cpp_handle_keyword(&variant.name),
                    constructor,
                    match_check,
                    fields,
                });
            }
            for field in ty_def.fields {
                let extern_mn = rust_file.add_field_assertions(&field, &ty_def.ty);
                let field = ZngurFieldData {
                    name: field.name,
                    ty: field.ty,
                    offset: match field.offset {
                        Some(offset) => ZngurFieldDataOffset::Offset(offset),
                        None => ZngurFieldDataOffset::Auto(
                            extern_mn.expect("auto offset did not provide extern name"),
                        ),
                    },
                };
                fields.push(field);
            }
            if let RustType::Tuple(fields) = &ty_def.ty {
                if !fields.is_empty() {
                    let rust_link_name = rust_file.add_tuple_constructor(&fields);
                    constructor = Some(CppFnSig {
                        rust_link_name,
                        inputs: fields
                            .iter()
                            .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                            .collect(),
                        output: ty.into_cpp(default_ns, &sanitized_crate_name),
                    });
                }
            }
            let is_unsized = ty_def
                .wellknown_traits
                .contains(&ZngurWellknownTrait::Unsized);
            for wellknown_trait in ty_def.wellknown_traits {
                let data = rust_file.add_wellknown_trait(&ty, wellknown_trait, is_unsized);
                wellknown_traits.push(data);
            }
            for method_details in ty_def.methods {
                let ZngurMethodDetails {
                    data: method,
                    use_path,
                    deref,
                    cpp_name,
                } = method_details;
                let rusty_inputs = real_inputs_of_method(&method, &ty);
                let cpp_name = cpp_name.as_ref().unwrap_or(&method.name);

                let sig = rust_file.add_function(
                    cpp_name,
                    &format!(
                        "<{}>::{}::<{}>",
                        deref.as_ref().map(|x| &x.0).unwrap_or(&ty),
                        method.name,
                        method.generics.iter().join(", "),
                    ),
                    &rusty_inputs,
                    &method.output,
                    method.is_safe,
                    use_path,
                    deref,
                    default_ns,
                    &sanitized_crate_name,
                );
                cpp_methods.push(CppMethod {
                    name: cpp_handle_keyword(cpp_name),
                    kind: method.receiver,
                    sig,
                });
            }
            cpp_file.type_defs.push(CppTypeDefinition {
                ty: ty.into_cpp(default_ns, &sanitized_crate_name),
                layout: rust_file.add_layout_policy_shim(&ty, layout),
                constructor,
                variants,
                discriminant,
                fields,
                methods: cpp_methods,
                wellknown_traits,
                cpp_heap_allocated: is_cpp_heap_allocated.then(|| CppHeapAllocatedData {
                    bridge_fn: rust_file.add_cpp_heap_allocated_bridge(ty),
                    cpp_type: cpp_name.expect("is_cpp_heap_allocated implies RustType::Cpp"),
                }),
                cpp_ref,
                cpp_stack_owned: cpp_stack_owned_data,
                from_trait: if let RustType::Boxed(b) = &ty {
                    if let RustType::Dyn(tr, _) = b.as_ref() {
                        if let RustTrait::Fn {
                            name,
                            inputs,
                            output,
                        } = tr
                        {
                            if let Entry::Vacant(e) = cpp_file.trait_defs.entry(tr.clone()) {
                                let rust_link_name =
                                    rust_file.add_builder_for_dyn_fn(name, inputs, output);
                                e.insert(CppTraitDefinition::Fn {
                                    sig: CppFnSig {
                                        rust_link_name,
                                        inputs: inputs
                                            .iter()
                                            .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                                            .collect(),
                                        output: output.into_cpp(default_ns, &sanitized_crate_name),
                                    },
                                });
                            }
                        }
                        Some(tr.clone())
                    } else {
                        None
                    }
                } else {
                    None
                },
                from_trait_ref: if let RustType::Dyn(tr, _) = &ty {
                    Some(tr.clone())
                } else {
                    None
                },
            });
        }
        // `c++::` types of imported modules are defined in their own crate (at
        // `::<crate>::generated::<path>`). Re-exporting them at the same relative path as local
        // `c++::` types lets this module's generated code refer to both the same way.
        let mut reexported = std::collections::HashSet::<&[String]>::new();
        for module in &zng.imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let module_crate = crate_name.replace('-', "_");
            for segs in &module.cpp_types {
                let Some((_, module_path)) = segs.split_last() else {
                    continue;
                };
                if !reexported.insert(segs.as_slice()) {
                    continue;
                }
                cpp_tree.insert(
                    module_path,
                    &format!(
                        "\n#[allow(unused_imports)]\npub use ::{module_crate}::generated::{};\n",
                        segs.join("::")
                    ),
                );
            }
        }
        rust_file.text.push_str(&cpp_tree.render());
        for func in zng.funcs {
            let sig = rust_file.add_function(
                &func.path.to_string(),
                &func.path.to_string(),
                &func.inputs,
                &func.output,
                func.is_safe,
                None,
                None,
                default_ns,
                &sanitized_crate_name,
            );
            cpp_file.fn_defs.push(CppFnDefinition {
                name: CppPath::from_rust_path(&func.path.path, default_ns, &sanitized_crate_name),
                sig,
            });
        }
        for func in zng.extern_cpp_funcs {
            let rust_link_name = rust_file.add_extern_cpp_function(
                &func.name,
                &func.inputs,
                &func.output,
                func.is_safe,
            );
            cpp_file.exported_fn_defs.push(CppExportedFnDefinition {
                name: cpp_handle_keyword(&func.name),
                sig: CppFnSig {
                    rust_link_name,
                    inputs: func
                        .inputs
                        .into_iter()
                        .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                        .collect(),
                    output: func.output.into_cpp(default_ns, &sanitized_crate_name),
                },
            });
        }
        for impl_block in zng.extern_cpp_impls {
            let rust_link_names = rust_file.add_extern_cpp_impl(
                &impl_block.ty,
                impl_block.tr.as_ref(),
                &impl_block.methods,
            );
            cpp_file.exported_impls.push(CppExportedImplDefinition {
                tr: impl_block
                    .tr
                    .map(|x| x.into_cpp(default_ns, &sanitized_crate_name)),
                ty: impl_block.ty.into_cpp(default_ns, &sanitized_crate_name),
                methods: impl_block
                    .methods
                    .iter()
                    .zip(&rust_link_names)
                    .map(|(method, link_name)| {
                        let inputs = real_inputs_of_method(method, &impl_block.ty);
                        let inputs = inputs
                            .iter()
                            .map(|ty| ty.into_cpp(default_ns, &sanitized_crate_name))
                            .collect();
                        (
                            cpp_handle_keyword(&method.name),
                            CppFnSig {
                                rust_link_name: link_name.clone(),
                                inputs,
                                output: method.output.into_cpp(default_ns, &sanitized_crate_name),
                            },
                        )
                    })
                    .collect(),
            });
        }
        let (h, cpp) = cpp_file.render(default_ns, &sanitized_crate_name);
        (rust_file.text, h, cpp)
    }
}

pub struct ZngHeaderGenerator {
    pub panic_to_exception: bool,
    pub cpp_namespace: String,
}

impl ZngHeaderGenerator {
    /// Renders the zngur.h header
    pub fn render(&self) -> Result<String, askama::Error> {
        let zng_h = ZngHeaderTemplate {
            panic_to_exception: self.panic_to_exception,
            cpp_namespace: self.cpp_namespace.clone(),
        };
        zng_h.render()
    }
}

fn real_inputs_of_method(method: &ZngurMethod, ty: &RustType) -> Vec<RustType> {
    let receiver_type = match method.receiver {
        ZngurMethodReceiver::Static => None,
        ZngurMethodReceiver::Ref(m) => Some(RustType::Ref(m, Box::new(ty.clone()))),
        ZngurMethodReceiver::Move => Some(ty.clone()),
    };
    let rusty_inputs = receiver_type
        .into_iter()
        .chain(method.inputs.clone())
        .collect::<Vec<_>>();
    rusty_inputs
}
#[cfg(test)]
mod tests {
    use zngur_def::*;

    use crate::ZngurGenerator;

    fn minimal_heap_allocated_type(ty: RustType) -> ZngurType {
        ZngurType {
            layout: Some(LayoutPolicy::HeapAllocated),
            trivially_relocatable: false,
            ..minimal_stack_owned_type(ty, 0, 1)
        }
    }

    /// A `c++::` type that Rust holds by value via `#layout` + `#trivially_relocatable`.
    fn minimal_stack_owned_type(ty: RustType, size: usize, align: usize) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            trivially_relocatable: true,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
        }
    }

    /// A `c++::` type that Rust only refers to.
    fn minimal_ref_type(ty: RustType) -> ZngurType {
        ZngurType {
            layout: None,
            trivially_relocatable: false,
            ..minimal_stack_owned_type(ty, 0, 1)
        }
    }

    fn adt(segments: &[&str]) -> RustType {
        RustType::Adt(RustPathAndGenerics {
            path: segments.iter().map(|s| s.to_string()).collect(),
            generics: vec![],
            named_generics: vec![],
        })
    }

    fn cpp(segments: &[&str]) -> RustType {
        RustType::Cpp(segments.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn cpp_type_gets_nested_module() {
        let spec = ZngurSpec {
            types: vec![minimal_ref_type(cpp(&["a", "b", "Name"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(rust_code.contains("pub struct Name(());"));
        assert!(!rust_code.contains("pub mod cpp {"));
        assert!(!h.contains("sizeof(::a::b::Name)"));
        assert!(h.contains("inline ::a::b::Name& cpp()"));
    }

    #[test]
    fn cpp_heap_allocated_bridge_references_correct_wrapper_path_for_cpp_type() {
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(cpp(&["a", "Name"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("*mut a::Name"));
        assert!(!rust_code.contains("*mut cpp::Name"));
        assert!(!rust_code.contains("*mut cpp::a::Name"));
        assert!(h.contains("inline ::a::Name& cpp()"));
        let (_, after_ctor) = h
            .split_once("inline Name(::rust::ZngurCppOpaqueOwnedObject&& o) noexcept {")
            .expect("missing ZngurCppOpaqueOwnedObject constructor");
        let (ctor_body, _) = after_ctor.split_once('}').unwrap();
        assert!(ctor_body.contains("__zngur_data ="));
    }

    #[test]
    fn cpp_stack_owned_destructor_names_match_and_are_unique() {
        let cases = [
            (cpp(&["a", "Point"]), "::a::Point"),
            (cpp(&["b", "Point"]), "::b::Point"),
            (cpp(&["a", "b", "Point"]), "::a::b::Point"),
            (cpp(&["a__b", "Point"]), "::a__b::Point"),
            (cpp(&["crate", "Point"]), "::crate::Point"),
            (cpp(&["crate__Point"]), "::crate__Point"),
        ];
        let spec = ZngurSpec {
            types: cases
                .iter()
                .map(|(ty, _)| minimal_stack_owned_type(ty.clone(), 16, 8))
                .collect(),
            mangling_base: "my_base".to_owned(),
            ..Default::default()
        };
        let rust_ref = crate::RustFile::new("my_base");
        let expected: Vec<(String, &str)> = cases
            .iter()
            .map(|(ty, cpp_ty)| {
                let name = rust_ref.cpp_stack_owned_destructor_fn(ty);
                assert!(name.contains("my_base"), "{name}");
                (name, *cpp_ty)
            })
            .collect();
        let unique_names: std::collections::HashSet<&str> =
            expected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            unique_names.len(),
            expected.len(),
            "destructor symbols collided: {expected:?}"
        );

        let (rust_code, _h, cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp = cpp.expect("stack owned types need a C++ file for their destructors");
        assert!(
            !rust_code.contains("pub fn _zngur_"),
            "unexpected safe wrapper emitted in:\n{rust_code}"
        );
        for (name, cpp_type) in &expected {
            assert!(
                rust_code.contains(&format!("fn {name}(i0: *mut u8);")),
                "{name} extern decl not in:\n{rust_code}"
            );
            assert!(
                rust_code.contains(&format!("{name}(self.buffer.get() as *mut _);")),
                "{name} destruct call not in:\n{rust_code}"
            );
            assert!(
                cpp.contains(&format!("void {name}(uint8_t* i0)")),
                "{name} C++ definition not in:\n{cpp}"
            );
            assert!(
                cpp.contains(&format!("using __zngur_T = {cpp_type};")),
                "type alias for {cpp_type} not in:\n{cpp}"
            );
        }
    }

    #[test]
    fn cpp_types_sharing_a_prefix_merge_into_a_single_module() {
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(cpp(&["a", "Foo"])),
                minimal_ref_type(cpp(&["a", "Bar"])),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Both structs must land inside a single, merged `pub mod a { ... }`.
        let mod_a_count = rust_code.matches("pub mod a {").count();
        assert_eq!(
            mod_a_count, 1,
            "expected exactly one `pub mod a {{` block, found {mod_a_count} in:\n{rust_code}"
        );
        assert!(rust_code.contains("pub struct Foo"));
        assert!(rust_code.contains("pub struct Bar"));
        assert!(!rust_code.contains("pub mod cpp {"));
    }

    #[test]
    fn cpp_std_and_core_modules_do_not_shadow_rust_prelude_paths() {
        let my_trait = RustTrait::Normal(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "MyTrait".to_owned()],
            generics: vec![],
            named_generics: vec![],
        });
        let mut traits = indexmap::IndexMap::new();
        traits.insert(
            my_trait.clone(),
            ZngurTrait {
                tr: my_trait.clone(),
                methods: vec![ZngurMethod {
                    name: "run".to_owned(),
                    generics: vec![],
                    receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                    inputs: vec![],
                    output: RustType::UNIT,
                    is_safe: None,
                }],
            },
        );
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(cpp(&["std", "String"]), 16, 8),
                minimal_stack_owned_type(cpp(&["core", "Widget"]), 16, 8),
                ZngurType {
                    ty: RustType::Boxed(Box::new(RustType::Dyn(my_trait.clone(), vec![]))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: false,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                },
                ZngurType {
                    ty: RustType::Ref(Mutability::Not, Box::new(RustType::Dyn(my_trait, vec![]))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: false,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                },
                ZngurType {
                    ty: RustType::Boxed(Box::new(RustType::Dyn(
                        RustTrait::Fn {
                            name: "Fn".to_owned(),
                            inputs: vec![],
                            output: Box::new(RustType::UNIT),
                        },
                        vec![],
                    ))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: false,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                },
                ZngurType {
                    ty: adt(&["crate", "MyEnum"]),
                    layout: Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
                    trivially_relocatable: false,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![ZngurVariant {
                        name: "Val".to_owned(),
                        fields: vec![ZngurField {
                            name: "x".to_owned(),
                            ty: RustType::Primitive(PrimitiveRustType::Int(32)),
                            offset: None,
                        }],
                        exhaustive: true,
                    }],
                    fields: vec![],
                },
            ],
            traits,
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod std {"));
        assert!(rust_code.contains("pub mod core {"));
        assert!(rust_code.contains("::core::cell::UnsafeCell<::core::mem::MaybeUninit<"));
        assert!(rust_code.contains("::core::marker::PhantomData<*mut ()>"));
        assert!(rust_code.contains("::core::marker::PhantomPinned"));
        assert_eq!(
            rust_code
                .matches("::std::ptr::write(o as *mut _, r)")
                .count(),
            3
        );
        assert!(rust_code.contains("::std::hint::unreachable_unchecked()"));
        let stripped = rust_code
            .replace("std::String", "")
            .replace("core::Widget", "")
            .replace("::std::", "")
            .replace("::core::", "");
        assert!(
            !stripped.contains("std::") && !stripped.contains("core::"),
            "found unqualified `std::` or `core::` in:\n{rust_code}"
        );
    }

    #[test]
    fn cpp_type_wrappers_live_in_rust_cpp_namespace_and_do_not_collide_with_rust_std() {
        let rust_string = ZngurType {
            trivially_relocatable: false,
            ..minimal_stack_owned_type(adt(&["std", "string", "String"]), 24, 8)
        };
        let spec = ZngurSpec {
            types: vec![rust_string, minimal_ref_type(cpp(&["std", "string"]))],
            ..Default::default()
        };
        let (_rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            h.contains("namespace rust {\nnamespace cpp {\nnamespace std {\nstruct string {"),
            "missing `rust::cpp::std::string` wrapper in:\n{h}"
        );
        assert!(
            h.contains("namespace rust {\nnamespace std {\nnamespace string {\nstruct String {"),
            "missing `rust::std::string::String` wrapper in:\n{h}"
        );
    }

    #[test]
    fn cpp_type_with_layout_and_trivially_relocatable_is_held_by_value() {
        let spec = ZngurSpec {
            types: vec![minimal_stack_owned_type(cpp(&["geo", "Point"]), 16, 8)],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod geo {"));
        assert!(rust_code.contains("pub struct Point {"));
        assert!(rust_code.contains("unsafe impl ::zngur_lib::ZngCppStackObject for Point {}"));
        assert!(h.contains(
            "static_assert(::rust::is_trivially_relocatable< ::geo::Point >::value, \"Type must \
             be trivially relocatable\");"
        ));
    }

    #[test]
    fn cpp_type_with_layout_but_not_trivially_relocatable_is_reference_only() {
        let spec = ZngurSpec {
            types: vec![ZngurType {
                trivially_relocatable: false,
                ..minimal_stack_owned_type(cpp(&["geo", "Point"]), 16, 8)
            }],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Zero-sized in Rust...
        assert!(rust_code.contains("pub struct Point(());"));
        assert!(!rust_code.contains("ZngCppStackObject"));
        // ...but the declared layout is still checked in C++.
        assert!(h.contains("static_assert(sizeof(::geo::Point) == 16, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Point) == 8, \"Alignment mismatch\");"));
    }

    #[test]
    fn imported_cpp_types_are_reexported_and_deduplicated() {
        let bar_import = ModuleImport {
            path: "./bar.zng".into(),
            crate_name: Some("bar-bindings".to_owned()),
            cpp_types: vec![
                vec!["ns".to_owned(), "Bar".to_owned()],
                vec!["Top".to_owned()],
            ],
        };
        let spec = ZngurSpec {
            // Duplicate `ModuleImport` entries (e.g. from two `merge`d files importing the same
            // module) must emit each `pub use` only once.
            imported_modules: vec![bar_import.clone(), bar_import],
            // A local `c++::` type in the same namespace shares the module.
            types: vec![minimal_ref_type(cpp(&["ns", "Local"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert_eq!(
            rust_code
                .matches("pub use ::bar_bindings::generated::Top;")
                .count(),
            1
        );
        let ns_module = rust_code
            .split("pub mod ns {")
            .nth(1)
            .expect("no `ns` module");
        assert_eq!(
            ns_module
                .matches("pub use ::bar_bindings::generated::ns::Bar;")
                .count(),
            1
        );
        assert!(ns_module.contains("pub struct Local"));
        assert_eq!(rust_code.matches("pub mod ns {").count(), 1);
        assert_eq!(h.matches("#include \"./bar.zng.h\"").count(), 1);
    }

    #[test]
    fn cpp_type_without_layout_is_reference_only_and_has_no_layout_checks() {
        let spec = ZngurSpec {
            types: vec![minimal_ref_type(cpp(&["geo", "Point"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub struct Point(());"));
        assert!(!rust_code.contains("ZngCppStackObject"));
        assert!(!h.contains("sizeof(::geo::Point)"));
        assert!(h.contains("inline ::geo::Point& cpp()"));
    }
}
