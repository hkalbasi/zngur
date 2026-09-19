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
use cpp::cpp_handle_keyword;
use indexmap::map::Entry;
use itertools::Itertools;
use rust::CoroSupport;
use rust::IntoCpp;

pub mod cpp;
mod rust;
mod template;

use askama::Template;
pub use rust::RustFile;
pub use zngur_parser::{ParseResult, ParsedZngFile, cfg};

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
        for module in &zng.imported_modules {
            cpp_file
                .additional_includes
                .push_str(&format!("\n#include \"{}.h\"", module.path.display()));
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
        let mut cpp_mod_content = String::new();
        let mut cpp_tree = CppModuleTree::default();
        cpp_file.coro_support = coro_support;
        for ty_def in zng.types {
            let ty = &ty_def.ty;
            let is_copy = ty_def.wellknown_traits.contains(&ZngurWellknownTrait::Copy);
            let Some(layout) = ty_def.layout else {
                unreachable!("Every type should have a defined layout policy before rendering")
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
            let (type_name, is_cpp, module_path): (String, bool, Vec<String>) = match &ty {
                RustType::Cpp(segs) => {
                    let (name, path) = segs
                        .split_last()
                        .expect("Cpp path must have at least one segment");
                    (name.clone(), true, path.to_vec())
                }
                _ => (
                    ty.to_string().split("::").last().unwrap().to_string(),
                    false,
                    vec![],
                ),
            };
            if let Some(cpp_stack_owned) = &ty_def.cpp_stack_owned {
                let destructor_name = format!("_zngur_crate_{type_name}_destructor");
                let mangled_name = rust_file.add_extern_cpp_function(
                    &destructor_name,
                    &[RustType::Ref(Mutability::Mut, Box::new(ty.clone()))],
                    &RustType::Tuple(vec![]),
                    true,
                );
                let size = cpp_stack_owned.size;
                let align = cpp_stack_owned.align;
                let struct_def = format!(
                    r#"
    #[repr(C)]
    #[repr(align({align}))]
    pub struct {type_name} {{
      pub(crate) buffer: core::cell::UnsafeCell<core::mem::MaybeUninit<[u8; {size}]>>,
      _no_auto_traits: core::marker::PhantomData<*mut ()>,
      _pinned: core::marker::PhantomPinned,
    }}

    unsafe impl ::zngur_lib::ZngCppObject for {type_name} {{}}
    unsafe impl ::zngur_lib::ZngCppStackObject for {type_name} {{}}

    unsafe impl ::zngur_lib::ZngCppDestruct for {type_name} {{
        unsafe fn destruct(&mut self) {{
            unsafe extern "C" {{
                fn {mangled_name}(i0: *mut u8, o: *mut u8);
            }}
            let mut dummy = ();
            unsafe {{
                {mangled_name}(self.buffer.get() as *mut _, &mut dummy as *mut () as *mut u8);
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
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_mod_content.push_str(&format!(
                        "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                    ));
                }
            }
            if ty_def.cpp_heap_allocated.is_some() {
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
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_mod_content.push_str(&format!(
                        "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                    ));
                }
            }
            if ty_def.cpp_ref.is_some() {
                let struct_def = format!(
                    r#"
    pub struct {type_name}(());
"#
                );
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_mod_content.push_str(&format!(
                        "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                    ));
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
                            .with_tail(variant.name.clone()),
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
                    name: variant.name,
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
                    use_path,
                    deref.map(|x| x.1),
                    default_ns,
                    &sanitized_crate_name,
                );
                cpp_methods.push(CppMethod {
                    name: cpp_handle_keyword(cpp_name).to_owned(),
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
                cpp_heap_allocated: ty_def.cpp_heap_allocated.map(|cpp_heap_allocated| {
                    CppHeapAllocatedData {
                        bridge_fn: rust_file.add_cpp_heap_allocated_bridge(&ty),
                        cpp_type: cpp_heap_allocated.0,
                    }
                }),
                cpp_ref: ty_def.cpp_ref,
                cpp_stack_owned: ty_def.cpp_stack_owned,
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
        if !cpp_mod_content.is_empty() {
            rust_file.text.push_str(&format!(
                r#"
pub mod cpp {{
{cpp_mod_content}
}}
"#
            ));
        }
        rust_file.text.push_str(&cpp_tree.render());
        for func in zng.funcs {
            let sig = rust_file.add_function(
                &func.path.to_string(),
                &func.path.to_string(),
                &func.inputs,
                &func.output,
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
                name: func.name.clone(),
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
                            cpp_handle_keyword(&method.name).to_owned(),
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
    pub fn render(&self) -> String {
        let zng_h = ZngHeaderTemplate {
            panic_to_exception: self.panic_to_exception,
            cpp_namespace: self.cpp_namespace.clone(),
        };
        zng_h.render().unwrap()
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

    fn minimal_heap_allocated_type(ty: RustType, cpp_path: &str) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::HeapAllocated),
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: Some(CppHeapAllocated(cpp_path.to_owned())),
            cpp_ref: None,
            cpp_stack_owned: None,
        }
    }

    fn minimal_stack_owned_type(
        ty: RustType,
        cpp_type: &str,
        size: usize,
        align: usize,
    ) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: Some(CppStackOwned {
                cpp_type: cpp_type.to_owned(),
                size,
                align,
            }),
        }
    }

    fn minimal_ref_type(ty: RustType, cpp_type: &str) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::ZERO_SIZED_TYPE),
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: Some(CppRef(cpp_type.to_owned())),
            cpp_stack_owned: None,
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
    fn old_style_type_gets_top_level_struct_and_deprecated_shim() {
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                adt(&["crate", "Way"]),
                "::osmium::Way",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Primary struct at top level, not nested under `mod cpp`:
        assert!(rust_code.contains("pub struct Way"));
        // Deprecated shim still present for backward compatibility:
        assert!(rust_code.contains("pub mod cpp {"));
        assert!(rust_code.contains("#[deprecated"));
        assert!(rust_code.contains("pub type Way = super::Way;"));
        // The heap-allocated bridge function must reference the bare
        // top-level name, not the old `cpp::Way` path (which would now
        // resolve to the deprecated shim and self-trigger a deprecation
        // warning under `-D warnings`).
        assert!(rust_code.contains("*mut Way"));
        assert!(!rust_code.contains("*mut cpp::Way"));
    }

    #[test]
    fn cpp_type_gets_nested_module_and_no_cpp_shim() {
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                cpp(&["a", "b", "Name"]),
                "::x::Name",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(rust_code.contains("pub struct Name"));
        // No `cpp` compatibility module should be emitted when there are no old-style types:
        assert!(!rust_code.contains("pub mod cpp {"));
        // The heap-allocated bridge function must reference the correct
        // nested path where the struct actually lives, not the old
        // `cpp::Name` path (which doesn't exist for a `RustType::Cpp` type
        // at all).
        assert!(rust_code.contains("*mut a::b::Name"));
        assert!(!rust_code.contains("*mut cpp::"));
    }

    #[test]
    fn cpp_heap_allocated_bridge_references_correct_wrapper_path_for_cpp_type() {
        // Regression test: add_cpp_heap_allocated_bridge (in rust.rs) used to
        // hardcode `cpp::{type_name}` for the bridge function's return/cast
        // type, which was only correct back when the wrapper struct
        // physically lived inside `mod cpp { ... }`. For `RustType::Cpp`
        // types there never was a `cpp::` home at all, so this was a
        // straight compile error waiting to happen once such a type used
        // #cpp_heap_allocated.
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                cpp(&["a", "Name"]),
                "::x::Name",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("*mut a::Name"));
        assert!(!rust_code.contains("*mut cpp::Name"));
        assert!(!rust_code.contains("*mut cpp::a::Name"));
    }

    #[test]
    fn mixed_old_and_new_style_types_both_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_heap_allocated_type(adt(&["crate", "Way"]), "::osmium::Way"),
                minimal_heap_allocated_type(cpp(&["a", "Name"]), "::x::Name"),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub struct Way"));
        assert!(rust_code.contains("pub mod cpp {"));
        assert!(rust_code.contains("pub type Way = super::Way;"));
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub struct Name"));
        // The `RustType::Cpp` type must NOT get a deprecated alias:
        assert!(!rust_code.contains("pub type Name = super::Name;"));
    }

    #[test]
    fn cpp_stack_owned_old_and_new_style_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(adt(&["crate", "Box2d"]), "::geos::Box2d", 16, 8),
                minimal_stack_owned_type(cpp(&["geo", "Point"]), "::geos::Point", 16, 8),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Old-style: top-level struct + deprecated shim.
        assert!(rust_code.contains("pub struct Box2d"));
        assert!(rust_code.contains("pub mod cpp {"));
        assert!(rust_code.contains("pub type Box2d = super::Box2d;"));
        // New-style: nested module, no shim.
        assert!(rust_code.contains("pub mod geo {"));
        assert!(rust_code.contains("pub struct Point"));
        assert!(!rust_code.contains("pub type Point = super::Point;"));
    }

    #[test]
    fn cpp_ref_old_and_new_style_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(adt(&["crate", "Handle"]), "::osmium::Handle"),
                minimal_ref_type(cpp(&["h", "Handle"]), "::x::Handle"),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Old-style: top-level struct + deprecated shim.
        assert!(rust_code.contains("pub struct Handle(());"));
        assert!(rust_code.contains("pub mod cpp {"));
        assert!(rust_code.contains("pub type Handle = super::Handle;"));
        // New-style: nested module, and only a single deprecated shim exists
        // overall (i.e. the `RustType::Cpp` variant did not also get one).
        assert!(rust_code.contains("pub mod h {"));
        assert_eq!(rust_code.matches("#[deprecated").count(), 1);
    }

    #[test]
    fn cpp_types_sharing_a_prefix_merge_into_a_single_module() {
        let spec = ZngurSpec {
            types: vec![
                minimal_heap_allocated_type(cpp(&["a", "Foo"]), "::x::Foo"),
                minimal_heap_allocated_type(cpp(&["a", "Bar"]), "::x::Bar"),
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
}
