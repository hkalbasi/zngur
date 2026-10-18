//! Rendering of the analyzed crate to Zngur `.zng` IDL output.
//!
//! The pipeline is split in two because of a lifetime constraint:
//!
//! ```text
//! (inside after_analysis, `TyCtxt` alive)          (any time after)
//! AutozngContext  ->  zngur_def::ZngurSpec   ->    IDLPrinter::print  ->  .zng text
//!                         build_spec()                 print_to_file()
//! ```
//!
//! The `rustc`/`rustc_public` types held by [`AutozngContext`] borrow from the rustc
//! `TyCtxt`, which only exists for the duration of the `after_analysis` callback.
//! Therefore [`build_spec`] **must** run while the `TyCtxt` is still in scope; the
//! resulting [`ZngurSpec`] is owned and lifetime-free, so [`print_to_file`] may run later.
//! Deciding *where* to write it (from `Config` and the crate name) is the driver's job.

use std::io::{BufWriter, Write};
use std::path::Path;

use indexmap::IndexMap;
use rustc_errors::DiagMessage;
use rustc_hash::FxHashSet;
use rustc_public::abi::FieldsShape;
use rustc_public::ty::{
    AdtDef, AssocKind, FieldDef, FnDef, GenericArgKind, GenericArgs, GenericParamDefKind, RigidTy,
    TraitDecl, TraitDef, Ty, TyKind,
};
use rustc_public::{CrateDef, CrateDefType, DefId};
use rustc_span::Span;
use zngur_def::printing::IDLPrinter;
use zngur_def::{
    LayoutPolicy, RustPathAndGenerics, RustTrait, RustType, TypeVar, ZngurField, ZngurFn,
    ZngurMethod, ZngurMethodDetails, ZngurMethodReceiver, ZngurSpec, ZngurTrait, ZngurType,
    ZngurVariant, ZngurWellknownTrait,
};

use crate::ctx::{
    AutozngContext, ExportedTraitRecord, FnRecord, ImplRecord, TraitFnRecord, TyRecord,
};
use crate::driver::with_ctxt;
use crate::error::{emit_warning_for_span, span_of_def, with_struct_warning_for_rustc_public_span};
use crate::metadata::{ExportName, IntoMetadataKey, MetadataKey};
use crate::rustc_ext::{
    AdtDefExt, EmitForExt, FnDefExt, RustcPublicTyExt, VariantDefExt, WellKnownTrait,
    WellKownTraitsExt,
};
use crate::type_map::{MapTypeError, is_type_block_less, map_rustc_public_ty_to_zngur};

/// Build the [`ZngurSpec`] from everything the analysis pass recorded into `ctx`.
///
/// Must be called inside the `after_analysis` callback (the context borrows the `TyCtxt`).
///
/// Emits one `type ... { .. }` block per recorded type (ADT or non-ADT, with its layout,
/// well-known traits, and — for ADTs — non-exhaustiveness, fields/variants and recorded
/// methods) plus one `fn` entry per recorded free function. Constructors and trait emission
/// depend on analysis work that is not wired up yet (see `build_type`).
pub fn build_spec(ctx: &AutozngContext) -> ZngurSpec {
    let mut types: Vec<ZngurType> = ctx
        .ty_records()
        .values()
        .filter_map(|record| build_type(ctx, record))
        .collect();

    // `ty_records` is a hash map; sort for stable output.
    types.sort_by_cached_key(|ty| ty.ty.to_string());

    let mut funcs: Vec<ZngurFn> = ctx
        .fn_records()
        .iter()
        .filter_map(|record| build_fn(ctx, record))
        .collect();

    // `fn_records` preserves discovery order; sort for stable output.
    funcs.sort_by_cached_key(|func| func.path.to_string());

    // One `trait ... { .. }` block per recorded trait, keyed by its `RustTrait` representation
    // (the `Fn*` family is collapsed; see `rust_trait_for`). Discovery order is deterministic,
    // but sort by the key's spelling for stable output like `types`/`funcs`.
    let mut traits: Vec<(RustTrait, ZngurTrait)> =
        ctx.trait_records().iter().filter_map(build_trait).collect();
    traits.sort_by_cached_key(|(key, _)| key.to_string());
    let traits: IndexMap<RustTrait, ZngurTrait> = traits.into_iter().collect();

    ZngurSpec {
        types,
        funcs,
        traits,
        ..ZngurSpec::default()
    }
}

/// Build one recorded trait into its `ZngurSpec::traits` entry: the [`RustTrait`] key plus the
/// [`ZngurTrait`] block (its associated functions as `.zng` methods).
///
/// Returns `None` when the trait cannot be represented (e.g. a const generic parameter).
fn build_trait(record: &ExportedTraitRecord) -> Option<(RustTrait, ZngurTrait)> {
    let key = rust_trait_for(record)?;
    let concrete = record.concrete.is_some();
    let methods = record
        .fns
        .iter()
        .filter_map(|trait_fn| build_trait_decl_method(trait_fn, concrete))
        .collect();
    let trt = ZngurTrait {
        tr: key.clone(),
        methods,
    };
    Some((key, trt))
}

/// The [`RustTrait`] representation of an exported trait, used as the key of
/// `ZngurSpec::traits`.
///
/// Most traits are just their full path ([`RustTrait::Normal`]). The `Fn`/`FnMut`/`FnOnce`
/// family is collapsed to [`RustTrait::Fn`] (its call signature) so codegen can build a
/// `std::function`-style bridge instead of a vtable. A concrete instantiation
/// (`emit_items(emit_traits = true)`) carries its concrete type arguments instead of type
/// variables.
fn rust_trait_for(record: &ExportedTraitRecord) -> Option<RustTrait> {
    let path = trait_path(record.decl.def_id);
    match fn_trait_name(&path) {
        Some(name) => Some(fn_trait_rust_trait(record, name)),
        None => {
            let generics = match &record.concrete {
                Some(trait_ref) => concrete_trait_generics(trait_ref)?,
                None => trait_decl_generics(&record.decl)?,
            };
            Some(RustTrait::Normal(RustPathAndGenerics {
                path,
                generics,
                named_generics: Vec::new(),
            }))
        }
    }
}

/// The concrete type arguments of a trait instantiation (`dyn GenericTrait<u32>` -> `[u32]`),
/// taken from the trait reference's arguments with the leading `Self` argument dropped.
///
/// Returns `None` when an argument cannot be mapped to a concrete `.zng` type (the whole trait is
/// then dropped by [`rust_trait_for`]).
fn concrete_trait_generics(trait_ref: &rustc_public::ty::TraitRef) -> Option<Vec<RustType>> {
    let mut generics = Vec::new();
    for arg in trait_ref.args().0.iter().skip(1) {
        match arg {
            // Lifetimes are dropped (rustc re-infers them); a const argument has no `.zng`
            // spelling, so the whole trait is dropped.
            GenericArgKind::Lifetime(_) => {}
            GenericArgKind::Type(ty) => {
                generics
                    .push(with_ctxt(|driver| map_rustc_public_ty_to_zngur(*ty, driver.tcx)).ok()?);
            }
            GenericArgKind::Const(_) => return None,
        }
    }
    Some(generics)
}

/// The trait's full path as `.zng` path segments: the local crate is spelled `crate`, external
/// crates keep their name (the printer adds the leading `::`).
fn trait_path(def: TraitDef) -> Vec<String> {
    let mut path: Vec<String> = def.name().split("::").map(Into::into).collect();
    if def.krate().is_local && !path.is_empty() {
        path[0] = "crate".to_string();
    }
    path
}

/// The name of the `Fn*` trait this path denotes, if any (`Fn`/`FnMut`/`FnOnce` in `std`/`core`).
fn fn_trait_name(path: &[String]) -> Option<String> {
    match path {
        [krate, ops, name] if (krate == "std" || krate == "core") && ops == "ops" => {
            matches!(name.as_str(), "Fn" | "FnMut" | "FnOnce").then(|| name.clone())
        }
        _ => None,
    }
}

/// The trait's own generic parameters as `.zng` type variables (`trait Foo<T>` -> `Foo<T>`).
///
/// Returns `None` for a const generic parameter, which the `.zng` IR cannot spell.
fn trait_decl_generics(decl: &TraitDecl) -> Option<Vec<RustType>> {
    let generics = decl.generics_of();
    let mut out = Vec::new();
    for param in &generics.params {
        // A trait's implicit `Self` is its first parameter; it is not a `.zng` generic argument.
        if generics.has_self && param.index == 0 {
            continue;
        }
        match param.kind {
            GenericParamDefKind::Lifetime => {}
            GenericParamDefKind::Type { .. } => {
                out.push(RustType::TypeVar(TypeVar(param.name.to_string())));
            }
            GenericParamDefKind::Const { .. } => return None,
        }
    }
    Some(out)
}

/// The `RustTrait::Fn` representation of a `Fn`/`FnMut`/`FnOnce` trait: its name plus the call
/// signature (`Args` -> `Output`).
///
/// A trait *declaration* has no concrete instantiation, so `Args` and `Output` are rendered as
/// type variables; a concrete `Fn*` signature comes from a `dyn`/`impl` trait reference (see
/// `type_map`). `Args` is the first type parameter after `Self`, `Output` the sole associated
/// type.
fn fn_trait_rust_trait(record: &ExportedTraitRecord, name: String) -> RustTrait {
    let decl = &record.decl;
    let generics = decl.generics_of();
    let inputs = generics
        .params
        .iter()
        .filter(|param| !(generics.has_self && param.index == 0))
        .filter_map(|param| match param.kind {
            GenericParamDefKind::Type { .. } => {
                Some(RustType::TypeVar(TypeVar(param.name.to_string())))
            }
            _ => None,
        })
        .collect();
    let output = decl
        .def_id
        .associated_items()
        .iter()
        .find_map(|item| match item.kind {
            AssocKind::Type { .. } => Some(RustType::TypeVar(TypeVar(
                item.def_id
                    .def_id()
                    .name()
                    .split("::")
                    .last()
                    .unwrap_or_default()
                    .to_owned(),
            ))),
            _ => None,
        })
        .unwrap_or(RustType::UNIT);
    RustTrait::Fn {
        name,
        inputs,
        output: Box::new(output),
    }
}

/// Build one of a trait declaration's associated functions into a `.zng` [`ZngurMethod`].
///
/// For a declaration (`concrete == false`) there is no concrete `Self`, so the method's identity
/// signature is used: `Self` (and any other generic parameter) appears as itself, and a method
/// whose signature mentions one is warn-and-skipped (the `.zng` IR has no type parameters). For a
/// concrete instantiation the method is instantiated with the trait reference's concrete
/// arguments, so `T` becomes the concrete type.
fn build_trait_decl_method(trait_fn: &TraitFnRecord, concrete: bool) -> Option<ZngurMethod> {
    let def = trait_fn.def;
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };
    let name = name.to_string();
    // The receiver is the trait's own `Self` parameter: the first argument.
    let self_ty = match trait_fn.generics.0.first() {
        Some(GenericArgKind::Type(ty)) => *ty,
        _ => return None,
    };

    // A concrete instantiation is only valid when the recorded arguments cover every generic
    // parameter of the method (the trait's own and the method's); otherwise its identity
    // signature is used and the method is skipped as an unsupported type parameter.
    let args = concrete
        .then_some(&trait_fn.generics)
        .filter(|args| args.0.len() == def.generics_len());
    if let Some(args) = args
        && !trait_fn.clauses.clone().instantiate(args).all_hold()
    {
        return None;
    }

    let signature = match args {
        Some(args) => def.instantiate_sig(args),
        None => def.fn_sig().skip_binder(),
    };
    let (receiver, inputs, output) = method_signature_parts(
        def,
        &name,
        *has_self,
        self_ty,
        None,
        &signature,
        trait_fn.span,
    )?;
    Some(ZngurMethod {
        name,
        generics: own_type_params(&def),
        receiver,
        inputs,
        output,
        is_safe: matches!(signature.safety, rustc_public::mir::Safety::Safe),
    })
}

/// The type parameters a function declares itself, as `.zng` type variables (`fn foo<T>` -> `[T]`).
///
/// `Generics::params` holds only the function's own parameters (the enclosing trait/impl's are
/// counted by `parent_count`), which is exactly what the `.zng` turbofish lists.
fn own_type_params(def: &FnDef) -> Vec<RustType> {
    def.generics_of()
        .params
        .iter()
        .filter_map(|param| match param.kind {
            GenericParamDefKind::Type { .. } => {
                Some(RustType::TypeVar(TypeVar(param.name.to_string())))
            }
            _ => None,
        })
        .collect()
}

/// Convert a single recorded type into a [`ZngurType`], or `None` if it cannot be represented
/// (with a warning emitted).
///
/// ADTs and non-ADT types alike are emitted as `type` blocks: `str`, slices (`[T]`), tuples
/// (`(A, B)`) and primitives are all valid `.zng` types. The only exception is the empty tuple
/// (`()`, unit), which zngur specialises and which must never be declared. Fields, variants and
/// methods only exist for ADTs; everything else contributes just its layout and well-known
/// traits.
fn build_type(ctx: &AutozngContext, record: &TyRecord) -> Option<ZngurType> {
    // A generic template (e.g. `GenericExample<T>`) has no `.zng` representation of its own;
    // only its concrete instantiations are emitted (typically enumerated by an `emit_items`
    // const). A type parameter is expected here, not an unsupported shape, so skip silently
    // rather than warn.
    if !record.is_concrete {
        return None;
    }

    let adt = record.adt;

    let rust_ty = match with_ctxt(|driver| map_rustc_public_ty_to_zngur(record.ty, driver.tcx)) {
        Ok(ty) => ty,
        Err(MapTypeError::Unsupported(msg)) => {
            // An ADT has a `DefId` to point the primary warning at, with an `emitted here` note at
            // the `emit` that selected it; an enumerated non-ADT (a slice, tuple, ...) has no
            // `DefId` of its own, so the `emit` span is the primary span instead.
            match adt {
                Some(adt) => warn(
                    adt.def_id(),
                    record.span,
                    format!("autozng: skipping `{}`: {msg}", record.ty),
                ),
                None => warn_emit_span(
                    record.span,
                    format!("autozng: skipping `{}`: {msg}", record.ty),
                ),
            }
            return None;
        }
        // An unresolved projection / higher-ranked parameter: skip silently.
        Err(MapTypeError::SkipSilently) => return None,
    };

    // The empty tuple (unit) is `RustType::UNIT`, a distinguished value in the `.zng` IR (the
    // implicit return type of a function); zngur specialises it and it is never a `type` block.
    // Numeric primitives are likewise translated to native C++ numeric types, so declaring them
    // as `type` blocks would generate broken C++; they are only usable in signatures.
    if is_type_block_less(&rust_ty) {
        return None;
    }

    // The IDL can only rename associated methods and free functions (`fn ... as <name>`), and
    // only an ADT can carry tool attributes / fields / variants.
    if let Some(adt) = adt {
        warn_unsupported_rename(ctx, &adt, adt.def_id(), &format!("type `{}`", adt.name()));
    }
    let (fields, variants) = match adt {
        // Enums carry their fields per variant; every other ADT is a single struct/union
        // "variant".
        Some(adt) if adt.kind().is_enum() => (Vec::new(), build_variants(ctx, record, &adt)),
        Some(adt) => (build_fields(ctx, record, &adt), Vec::new()),
        None => (Vec::new(), Vec::new()),
    };

    let mut methods = build_methods(ctx, record);
    // `#[autozng::emit_on_implementers]` traits/impls contribute their functions to every type
    // they apply to, as if the functions had been enumerated in an `emit_items` const.
    methods.extend(build_emit_for_methods(ctx, record));

    Some(ZngurType {
        ty: rust_ty,
        // An explicit `#[autozng::layout(...)]` replaces the automatic `StackAllocated` policy
        // derived from the type's real size and alignment.
        layout: match record.adt.and_then(|adt| ctx.layout_override_for(&adt)) {
            Some(over) => Some(over.policy),
            None => record.layout.and_then(layout_policy),
        },
        wellknown_traits: wellknown_traits(record.ty),
        // Only ADTs can be `#[non_exhaustive]`; every other type is exhaustive.
        exhaustive: adt.is_none_or(|adt| adt.is_exhaustive()),
        methods,
        // TODO(render): constructors and the well-known-trait data payloads
        // (pretty_print/debug_print/drop_in_place) still need analysis work.
        constructor: None,
        variants,
        fields,
        cpp_heap_allocated: None,
        cpp_ref: None,
        cpp_stack_owned: None,
    })
}

/// Build the `field` entries for a struct/union.
///
/// `record.fields` holds the fields the analysis pass selected (visibility + `emit`/`skip`);
/// the ADT's single variant supplies their declaration order and the layout their offsets.
/// Fields whose type cannot be represented are warned about and omitted.
fn build_fields(ctx: &AutozngContext, record: &TyRecord, adt: &AdtDef) -> Vec<ZngurField> {
    let offsets = struct_field_offsets(record);
    let Some(variant) = adt.variants().into_iter().next() else {
        return Vec::new();
    };
    variant
        .fields()
        .into_iter()
        .enumerate()
        .filter_map(|(index, field)| {
            let emit_span = record.fields.get(&field.def_id())?.span;
            let offset = offsets
                .as_ref()
                .and_then(|offsets| offsets.get(index).copied());
            build_field(ctx, adt, &field, emit_span, offset)
        })
        .collect()
}

/// Build the `variant` entries for an enum.
///
/// Every variant is emitted; each carries the fields the analysis pass selected. Variant field
/// offsets are left to Zngur (`auto`) because they are variant-relative.
fn build_variants(ctx: &AutozngContext, record: &TyRecord, adt: &AdtDef) -> Vec<ZngurVariant> {
    adt.variants()
        .into_iter()
        .map(|variant| ZngurVariant {
            name: variant.name().to_string(),
            fields: variant
                .fields()
                .into_iter()
                .filter_map(|field| {
                    let emit_span = record.fields.get(&field.def_id())?.span;
                    build_field(ctx, adt, &field, emit_span, None)
                })
                .collect(),
            exhaustive: variant.is_exhaustive(),
        })
        .collect()
}

/// Map one selected field to the `.zng` IR, warning (and returning `None`) when its type cannot
/// be represented so only that field is dropped.
fn build_field(
    ctx: &AutozngContext,
    adt: &AdtDef,
    field: &FieldDef,
    emit_span: Span,
    offset: Option<usize>,
) -> Option<ZngurField> {
    // Fields are never renamed: the IDL only supports `fn ... as <name>`.
    let name = field.name.to_string();
    warn_unsupported_rename(
        ctx,
        field,
        field.def_id(),
        &format!("field `{name}` of `{}`", adt.name()),
    );
    let field_ty = field.ty();
    match with_ctxt(|driver| map_rustc_public_ty_to_zngur(field_ty, driver.tcx)) {
        Ok(ty) => Some(ZngurField { name, ty, offset }),
        Err(MapTypeError::Unsupported(msg)) => {
            warn(
                field.def_id(),
                emit_span,
                format!(
                    "autozng: skipping field `{name}` of `{}`: {msg}",
                    adt.name()
                ),
            );
            None
        }
        Err(MapTypeError::SkipSilently) => None,
    }
}

/// Field offsets for a struct/union, in declaration order.
///
/// Returns `None` when the layout has no precomputed per-field offsets (unions) or none is
/// available (e.g. a generic ADT); Zngur then computes the offsets itself (`auto`).
fn struct_field_offsets(record: &TyRecord) -> Option<Vec<usize>> {
    let layout = record.layout?;
    match layout.shape().fields {
        FieldsShape::Arbitrary { offsets } => {
            Some(offsets.iter().map(|offset| offset.bytes()).collect())
        }
        FieldsShape::Union(_) | FieldsShape::Primitive | FieldsShape::Array { .. } => None,
    }
}

/// Map a type's in-memory layout to a Zngur layout policy.
///
/// Unsized types have no stack layout and are emitted without a `#layout` directive.
fn layout_policy(layout: rustc_public::abi::Layout) -> Option<LayoutPolicy> {
    let shape = layout.shape();
    if shape.is_unsized() {
        return None;
    }
    Some(LayoutPolicy::StackAllocated {
        size: shape.size.bytes(),
        align: shape.abi_align as usize,
    })
}

/// The subset of well-known traits that `zngur-def` understands.
fn wellknown_traits(ty: Ty) -> Vec<ZngurWellknownTrait> {
    ty.implimented_wellknown_traits()
        .into_iter()
        .filter_map(|(trait_kind, _)| match trait_kind {
            WellKnownTrait::Debug => Some(ZngurWellknownTrait::Debug),
            WellKnownTrait::Copy => Some(ZngurWellknownTrait::Copy),
            WellKnownTrait::Unsized => Some(ZngurWellknownTrait::Unsized),
            WellKnownTrait::Drop => Some(ZngurWellknownTrait::Drop),
            _ => None,
        })
        .collect()
}

/// Warn when a recorded `#[autozng::rename]` targets an item the IDL cannot rename.
///
/// The `.zng` syntax only renames associated methods, through `ZngurMethodDetails::cpp_name`
/// (`fn <rust_name>(...) -> ... as <cpp_name>`). A `rename`/`rename_all` on anything else
/// (type, variant, field, trait, free function) is accepted but has no effect, so the rendered
/// name is left untouched. The warning points at the item and adds a `renamed here` note at the
/// attribute that caused it.
fn warn_unsupported_rename<T: IntoMetadataKey + ?Sized>(
    ctx: &AutozngContext,
    key: &T,
    def_id: DefId,
    item: &str,
) {
    warn_unsupported_rename_with(ctx.export_name_for(key), def_id, item);
}

/// Like [`warn_unsupported_rename`], but for a caller that already resolved the export name
/// (e.g. a free function whose `rename` came from an `emit_items` monomorphization).
fn warn_unsupported_rename_with(export: Option<ExportName>, def_id: DefId, item: &str) {
    let Some(export) = export else {
        return;
    };
    with_struct_warning_for_rustc_public_span(
        def_id.span(),
        format!("autozng: ignoring `rename` on {item}: only associated methods can be renamed"),
        |mut diag, _tcx| {
            diag.span_note(export.span, "renamed here");
            diag.emit()
        },
    );
}

/// The export name recorded for one monomorphization of `def` (`#[autozng::rename]` on a `let`
/// binding inside an `emit_items` const).
///
/// The name is keyed by the fn's `DefId` plus its enumerated generic args (the same list stored
/// as `FnRecord::partial_args` / `TraitFnRecord::generics`), so it only applies to that
/// instantiation and not to every monomorphization of the same `FnDef`. An empty arg list is the
/// plain `DefId` key (see [`MetadataKey::for_monomorphization`]).
fn monomorphization_export_name(
    ctx: &AutozngContext,
    def: FnDef,
    args: Option<&GenericArgs>,
) -> Option<ExportName> {
    let args = args.filter(|args| !args.0.is_empty())?;
    ctx.export_name_for(&MetadataKey::for_monomorphization(def.def_id(), args))
}

/// Build the methods recorded for a type (from its inherent impl blocks).
///
/// For a fully monomorphized generic type (e.g. `Container<i32>`) the methods live on the ADT
/// *definition*'s record: the analysis pass walks an ADT's inherent impls once, onto the
/// generic self type. They are looked up there and instantiated with the concrete arguments of
/// `record.ty` (see [`build_method`]).
fn build_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<ZngurMethodDetails> {
    // The generic arguments of the fully monomorphized `record.ty`, if any. `None` for a type
    // with no generic arguments; the methods then render uninstantiated.
    let args = concrete_ty_args(record);

    let records = ctx.ty_records();
    let impls: Vec<&ImplRecord> = if !record.impls.is_empty() {
        // A type whose impls were recorded directly (a non-generic type, or a definition the
        // analysis pass walked).
        record.impls.values().collect()
    } else if let Some(adt) = record.adt {
        // A concrete instance: its impls were recorded on the ADT definition's record.
        records
            .get(&adt.ty().base_ty())
            .map(|definition| definition.impls.values().collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Methods with an explicitly-enumerated monomorphization (an `emit_items` const naming
    // `Self::foo::<T>`) are emitted as those monomorphizations; the uninstantiated record of the
    // same method is a template that cannot be rendered on its own, so it is skipped silently
    // (like a generic template *type*) rather than warned about.
    let monomorphized: Vec<FnDef> = impls
        .iter()
        .flat_map(|impl_record| impl_record.methods.iter())
        .filter(|method| method.partial_args.is_some())
        .map(|method| method.def)
        .collect();

    let mut methods: Vec<ZngurMethodDetails> = impls
        .into_iter()
        .flat_map(|impl_record| {
            impl_record
                .methods
                .iter()
                .map(move |method| (impl_record, method))
        })
        .filter_map(|(impl_record, method)| {
            if method.partial_args.is_none() && monomorphized.contains(&method.def) {
                return None;
            }
            build_method(ctx, record, impl_record, method, args.as_ref())
        })
        .collect();

    // Trait methods attached to the type (from an `emit_items` const naming e.g.
    // `Example::to_string`) are emitted alongside the inherent ones.
    methods.extend(build_trait_methods(ctx, record));

    methods
}

/// The generic arguments of a fully monomorphized `record.ty`, taken from the `RigidTy::Adt`
/// variant of its `TyKind`. Returns `None` when the type carries no generic arguments.
fn concrete_ty_args(record: &TyRecord) -> Option<GenericArgs> {
    match record.ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(_, args)) if !args.0.is_empty() => Some(args),
        _ => None,
    }
}

/// The turbofish generic arguments to print for `fn_def` (`fn foo<u8>(..)`), given the args
/// available for it (`None` when it was not monomorphized).
///
/// Only the arguments the function *owns* are printed: the ones inherited from the enclosing
/// impl block / trait are already named by the type the method hangs off (e.g. `Generic<u64>`),
/// so the first `generics_of(fn_def).parent_count` arguments are skipped. Arguments with no
/// `.zng` representation (a type parameter, a const generic, a projection) are dropped.
fn own_generic_args(fn_def: FnDef, args: Option<&GenericArgs>) -> Vec<RustType> {
    let Some(args) = args else {
        return Vec::new();
    };
    args.0
        .iter()
        .skip(fn_def.generics_of().parent_count)
        .filter_map(|arg| match arg {
            GenericArgKind::Type(ty) => {
                with_ctxt(|driver| map_rustc_public_ty_to_zngur(*ty, driver.tcx).ok())
            }
            _ => None,
        })
        .collect()
}

/// The concrete arguments to instantiate a method with.
///
/// `partial_args` are the method's own explicitly-enumerated args (from an `emit_items` const;
/// `None` for the ordinary impl walk). Their leading `generics_of(def).parent_count` entries
/// stand for the enclosing impl's parameters, so those are taken from `instance_args` (the
/// concrete `Self`'s arguments) instead, making the result fully concrete. Returns `None` unless
/// the result covers every generic parameter of `def`.
fn combine_method_args(
    def: &FnDef,
    partial_args: Option<&GenericArgs>,
    instance_args: Option<&GenericArgs>,
) -> Option<GenericArgs> {
    let parent_count = def.generics_of().parent_count;
    let mut combined: Vec<GenericArgKind> = Vec::new();
    match (partial_args, instance_args) {
        (Some(partial), _) => {
            // The impl's parameters come from the concrete instance; only the method's *own*
            // args (past `parent_count`) are taken from `partial`.
            if let Some(instance) = instance_args {
                combined.extend(
                    instance.0[..parent_count.min(instance.0.len())]
                        .iter()
                        .cloned(),
                );
            }
            combined.extend(
                partial.0[parent_count.min(partial.0.len())..]
                    .iter()
                    .cloned(),
            );
        }
        (None, Some(instance)) => combined.extend(instance.0.iter().cloned()),
        (None, None) => return None,
    }
    (combined.len() == def.generics_len()).then_some(GenericArgs(combined))
}

fn build_method(
    ctx: &AutozngContext,
    record: &TyRecord,
    impl_record: &ImplRecord,
    method: &FnRecord,
    instance_args: Option<&GenericArgs>,
) -> Option<ZngurMethodDetails> {
    let def = method.def;
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };
    // The IDL keeps the original Rust name (needed to construct the bindings); a `rename` goes
    // into `cpp_name` as the `fn ... as <name>` suffix.
    let name = name.to_string();
    // A `rename` on the method declaration is keyed by the `AssocItem`; one on a `let` binding is
    // keyed by the named monomorphization, so it is looked up first and only applies to it.
    let cpp_name = monomorphization_export_name(ctx, def, method.partial_args.as_ref())
        .or_else(|| ctx.export_name_for(&assoc_item))
        .map(|export| export.name);
    let self_ty = record.ty;
    // The ADT definition's (identity) self type, e.g. `Generic<T>` for a `Generic<i32>` record.
    // Used to still recognize the receiver of a method whose own generics prevent instantiation.
    let self_def_ty = record.adt.map(|adt| adt.ty().base_ty());

    // The concrete arguments to instantiate `def` with: the method's own explicitly-enumerated
    // args (when an `emit_items` const named it) with the impl's parameters taken from the
    // concrete instance. `None` when they do not cover every generic parameter of the method, in
    // which case its identity signature is rendered and skipped as an unsupported parameter.
    let combined_args = combine_method_args(&def, method.partial_args.as_ref(), instance_args);
    let args = combined_args.as_ref();

    if let Some(args) = args {
        // The clauses of both the impl block and the function itself must hold for the concrete
        // arguments, or the method would be emitted with unmet bounds.
        if !impl_record.clauses.clone().instantiate(args).all_hold()
            || !method.clauses.clone().instantiate(args).all_hold()
        {
            return None;
        }
    }

    let signature = match args {
        Some(args) => def.instantiate_sig(args),
        None => def.fn_sig().skip_binder(),
    };

    let (receiver, inputs, output) = method_signature_parts(
        def,
        &name,
        *has_self,
        self_ty,
        self_def_ty,
        &signature,
        method.span,
    )?;

    Some(ZngurMethodDetails {
        data: ZngurMethod {
            name,
            generics: own_generic_args(def, method.partial_args.as_ref()),
            receiver,
            inputs,
            output,
            is_safe: matches!(signature.safety, rustc_public::mir::Safety::Safe),
        },
        use_path: None,
        deref: None,
        cpp_name,
    })
}

/// The receiver, inputs and output of an instantiated method/function signature.
///
/// Classifies the receiver from the first parameter; receivers like `Box<Self>`, `Rc<Self>`,
/// `Pin<&Self>`, ... are not representable in the `.zng` IR yet, so those items are warned
/// about and skipped. Inputs are mapped with `has_self` skipped; a warning is emitted when the
/// signature cannot be represented.
fn method_signature_parts(
    def: FnDef,
    name: &str,
    has_self: bool,
    self_ty: Ty,
    self_def_ty: Option<Ty>,
    signature: &rustc_public::ty::FnSig,
    emit_span: Span,
) -> Option<(ZngurMethodReceiver, Vec<RustType>, RustType)> {
    let receiver = if has_self {
        let first_input = signature.inputs().first().copied();
        match first_input.map(|ty| ty.kind()) {
            Some(TyKind::RigidTy(RigidTy::Ref(_, inner, mutability)))
                if inner == self_ty || Some(inner) == self_def_ty =>
            {
                ZngurMethodReceiver::Ref(match mutability {
                    rustc_public::mir::Mutability::Mut => zngur_def::Mutability::Mut,
                    rustc_public::mir::Mutability::Not => zngur_def::Mutability::Not,
                })
            }
            // `self` by value: the first parameter is exactly the self type. Compared against
            // the instantiated signature (and the ADT definition, for uninstantiated methods)
            // so concrete generic instances like `Container<i32>` are recognized, and against
            // the trait's own `Self` parameter for a trait declaration.
            _ if first_input == Some(self_ty) || first_input == self_def_ty => {
                ZngurMethodReceiver::Move
            }
            other => {
                warn(
                    def.def_id(),
                    emit_span,
                    format!("autozng: skipping method `{name}`: unsupported receiver `{other:?}`"),
                );
                return None;
            }
        }
    } else {
        ZngurMethodReceiver::Static
    };

    let inputs = with_ctxt(|driver| {
        signature
            .inputs()
            .iter()
            .skip(usize::from(has_self))
            .map(|input| map_rustc_public_ty_to_zngur(*input, driver.tcx))
            .collect::<Result<Vec<_>, _>>()
    });
    let (inputs, output) = match (
        inputs,
        with_ctxt(|driver| map_rustc_public_ty_to_zngur(signature.output(), driver.tcx)),
    ) {
        (Ok(inputs), Ok(output)) => (inputs, output),
        (Err(MapTypeError::Unsupported(msg)), _) | (_, Err(MapTypeError::Unsupported(msg))) => {
            warn(
                def.def_id(),
                emit_span,
                format!("autozng: skipping method `{name}`: {msg}"),
            );
            return None;
        }
        // An unresolved projection / higher-ranked parameter: skip silently.
        (Err(MapTypeError::SkipSilently), _) | (_, Err(MapTypeError::SkipSilently)) => {
            return None;
        }
    };

    Some((receiver, inputs, output))
}

/// Build the trait methods attached to a type (see [`TraitFnRecord`]).
fn build_trait_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<ZngurMethodDetails> {
    record
        .trait_fns
        .iter()
        .filter_map(|trait_fn| build_trait_method(ctx, record, trait_fn))
        .collect()
}

/// Build the functions contributed to `record` by `#[autozng::emit_on_implementers]` records.
///
/// Each recorded trait / impl is resolved against the type: when the type implements the trait
/// and the record's clauses hold, the trait's functions are emitted on the type (with the trait
/// as their `use_path`), exactly as if they had been named in an `emit_items` const.
///
/// A function already emitted on the type (because it was explicitly enumerated in an
/// `emit_items` const, or contributed by another record) is skipped, so an explicit enumeration
/// and `emit_on_implementers` do not produce duplicate entries.
fn build_emit_for_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<ZngurMethodDetails> {
    let mut seen: FxHashSet<FnDef> = record
        .trait_fns
        .iter()
        .map(|trait_fn| trait_fn.def)
        .collect();
    let mut methods = Vec::new();

    for emit_for in ctx.emit_for_records().iter() {
        let Some(trait_ref) =
            record
                .ty
                .resolve_emit_for(emit_for.trait_def, emit_for.impl_def, &emit_for.clauses)
        else {
            continue;
        };
        for trait_fn in emit_for.fns.iter() {
            if !seen.insert(trait_fn.def) {
                continue;
            }
            // Instantiate the trait function with the concrete `Self: Trait<..>` arguments, so
            // `Self` becomes the type and any trait arguments are concrete.
            let instantiated = TraitFnRecord {
                def: trait_fn.def,
                trait_def: emit_for.trait_def,
                trait_ref: Some(trait_ref.clone()),
                generics: trait_ref.args().clone(),
                clauses: trait_fn.clauses.clone(),
                span: emit_for.span,
            };
            if let Some(method) = build_trait_method(ctx, record, &instantiated) {
                methods.push(method);
            }
        }
    }

    methods
}

/// Build one trait method into a [`ZngurMethodDetails`], or `None` if it cannot be represented
/// (with a warning emitted) or its clauses do not hold for the concrete arguments.
///
/// Mirrors [`build_method`], but the method belongs to a trait: the trait's path becomes the
/// `use_path` (`fn to_string(&self) -> ::std::string::String use ::std::string::ToString;`).
fn build_trait_method(
    ctx: &AutozngContext,
    record: &TyRecord,
    trait_fn: &TraitFnRecord,
) -> Option<ZngurMethodDetails> {
    let def = trait_fn.def;
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };
    let name = name.to_string();
    // See `build_method`: a `rename` on the `let` binding is keyed by the named monomorphization.
    let cpp_name = monomorphization_export_name(ctx, def, Some(&trait_fn.generics))
        .or_else(|| ctx.export_name_for(&assoc_item))
        .map(|export| export.name);
    let self_ty = record.ty;
    let self_def_ty = record.adt.map(|adt| adt.ty().base_ty());

    // The named generic args (Self first) must cover every generic parameter of the trait
    // method; the trait's clauses must then hold for them, or the method would be emitted with
    // unmet bounds.
    let args = (trait_fn.generics.0.len() == def.generics_len()).then_some(&trait_fn.generics);
    if let Some(args) = args
        && !trait_fn.clauses.clone().instantiate(args).all_hold()
    {
        return None;
    }

    let signature = match args {
        Some(args) => def.instantiate_sig(args),
        None => def.fn_sig().skip_binder(),
    };

    let (receiver, inputs, output) = method_signature_parts(
        def,
        &name,
        *has_self,
        self_ty,
        self_def_ty,
        &signature,
        trait_fn.span,
    )?;

    // The trait is rendered as the method's `use_path`.
    let trait_def = trait_fn.trait_def;
    let mut use_path: Vec<String> = trait_def.name().split("::").map(Into::into).collect();
    if trait_def.krate().is_local && !use_path.is_empty() {
        use_path[0] = "crate".to_string();
    }

    Some(ZngurMethodDetails {
        data: ZngurMethod {
            name,
            generics: own_generic_args(def, Some(&trait_fn.generics)),
            receiver,
            inputs,
            output,
            is_safe: matches!(signature.safety, rustc_public::mir::Safety::Safe),
        },
        use_path: Some(use_path),
        deref: None,
        cpp_name,
    })
}

/// Build a free function into a [`ZngurFn`], or `None` if it cannot be represented (with a
/// warning emitted).
///
/// Free functions named through `FnDef` cannot carry a `rename`: the IDL's `ZngurFn` has no
/// `cpp_name` slot, so a recorded `rename` is ignored with a warning.
fn build_fn(ctx: &AutozngContext, record: &FnRecord) -> Option<ZngurFn> {
    let fn_def = record.def;

    // A free function enumerated as a monomorphization (`foo::<u8>`) is instantiated with the
    // named arguments, mirroring methods: the args must cover every generic parameter, and every
    // recorded clause must then hold, or the function is dropped silently.
    let args = record
        .partial_args
        .as_ref()
        .filter(|args| args.0.len() == fn_def.generics_len());

    if let Some(args) = args
        && !record.clauses.clone().instantiate(args).all_hold()
    {
        return None;
    }

    let signature = match args {
        Some(args) => fn_def.instantiate_sig(args),
        None => fn_def.fn_sig().skip_binder(),
    };

    warn_unsupported_rename_with(
        monomorphization_export_name(ctx, fn_def, record.partial_args.as_ref())
            .or_else(|| ctx.export_name_for(&fn_def)),
        fn_def.def_id(),
        &format!("fn `{}`", fn_def.name()),
    );

    let inputs = with_ctxt(|driver| {
        signature
            .inputs()
            .iter()
            .map(|input| map_rustc_public_ty_to_zngur(*input, driver.tcx))
            .collect::<Result<Vec<_>, _>>()
    });
    let (inputs, output) = match (
        inputs,
        with_ctxt(|driver| map_rustc_public_ty_to_zngur(signature.output(), driver.tcx)),
    ) {
        (Ok(inputs), Ok(output)) => (inputs, output),
        (Err(MapTypeError::Unsupported(msg)), _) | (_, Err(MapTypeError::Unsupported(msg))) => {
            warn(
                fn_def.def_id(),
                record.span,
                format!("autozng: skipping fn `{}`: {msg}", fn_def.name()),
            );
            return None;
        }
        (Err(MapTypeError::SkipSilently), _) | (_, Err(MapTypeError::SkipSilently)) => {
            return None;
        }
    };

    let mut path: Vec<String> = fn_def.name().split("::").map(Into::into).collect();
    if fn_def.krate().is_local && !path.is_empty() {
        // Local paths are marked with a leading `crate` segment (see `type_map`).
        path[0] = "crate".to_string();
    }

    Some(ZngurFn {
        path: RustPathAndGenerics {
            path,
            generics: own_generic_args(fn_def, record.partial_args.as_ref()),
            named_generics: Vec::new(),
        },
        inputs,
        output,
    })
}

/// Print `spec` as `.zng` IDL to `path`, creating parent directories as needed.
pub fn print_to_file(spec: &ZngurSpec, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    {
        let mut printer = IDLPrinter::new(&mut writer);
        printer.write_idl(spec)?;
    }
    writer.flush()
}

/// Emit a warning attached to `def_id`'s span, with an `emitted here` note at `emit_span` (the
/// `#[autozng::emit]` that selected the item).
///
/// The `DefId` should be the type/ADT (`TyRecord::adt`, falling back to `TyRecord::ty`'s
/// `adt_def`) or the exact function definition / impl block the warning is about, so the
/// diagnostic points at the offending source location.
///
/// Only valid while the analysis callback is running (it needs the live `TyCtxt`).
fn warn(def_id: DefId, emit_span: Span, msg: impl Into<DiagMessage>) {
    let item_span = span_of_def(def_id);
    with_struct_warning_for_rustc_public_span(def_id.span(), msg, |mut diag, _tcx| {
        // Point at the emission site only when it is not the item itself (an `emit_items`
        // enumeration or a `let` binding); otherwise the note would just repeat the primary span.
        if !emit_span.is_dummy() && !item_span.contains(emit_span) {
            diag.span_note(emit_span, "emitted here");
        }
        diag.emit()
    });
}

/// Emit a warning for an item with no `DefId` of its own (e.g. an enumerated slice or tuple): the
/// `#[autozng::emit]` / `emit_items` that selected it is the primary span.
///
/// Only valid while the analysis callback is running (it needs the live `TyCtxt`).
fn warn_emit_span(emit_span: Span, msg: impl Into<DiagMessage>) {
    emit_warning_for_span(emit_span, msg);
}
