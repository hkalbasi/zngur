//! Rendering of the analyzed crate to Zngur IDL output.
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
use rustc_hash::{FxHashMap, FxHashSet};
use rustc_public::abi::FieldsShape;
use rustc_public::rustc_internal::{internal, stable};
use rustc_public::ty::{
    AdtDef, AssocKind, FieldDef, FnDef, GenericArgKind, GenericArgs, GenericParamDefKind, RigidTy,
    TraitDef, TraitRef, Ty, TyKind,
};
use rustc_public::{CrateDef, CrateDefType, DefId};
use rustc_span::Span;
use zngur_def::printing::IDLPrinter;
use zngur_def::{
    LayoutPolicy, RustPathAndGenerics, RustTrait, RustType, TypeVar, ZngurConstructor, ZngurField,
    ZngurFn, ZngurMethod, ZngurMethodDetails, ZngurMethodReceiver, ZngurSpec, ZngurTrait,
    ZngurType, ZngurVariant, ZngurWellknownTrait,
};

use crate::ctx::{
    AutozngContext, ExportedTraitRecord, FnRecord, ImplRecord, TraitFnRecord, TyRecord,
    combine_method_args, receiver_kind, visibility_kind,
};
use crate::driver::with_ctxt;
use crate::error::{emit_warning_for_span, span_of_def, with_struct_warning_for_rustc_public_span};
use crate::metadata::{ExportName, IntoMetadataKey, MetadataKey};
use crate::rustc_ext::{
    AdtDefExt, EmitForExt, FnDefExt, RustcPublicTyExt, VariantDefExt, VisibilityExt,
    WellKnownTrait, WellKnownTraitsExt, fn_trait_name, visible_path,
};
use crate::type_map::{MapTypeError, is_type_block_less, map_rustc_public_ty_to_zngur};
use crate::walk::{ReceiverKind, VisitReason, WalkIntent, WalkItem, WalkItemKind, WalkParts};

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
    // but sort by the key's string form for stable output like `types`/`funcs`.
    let mut traits: Vec<(RustTrait, ZngurTrait)> =
        ctx.trait_records().iter().filter_map(build_trait).collect();
    traits.sort_by_cached_key(|(key, _)| key.to_string());
    let traits: IndexMap<RustTrait, ZngurTrait> = traits.into_iter().collect();

    // A `trait_path` selector that never matched an emitted trait method is almost always a typo.
    for spec in ctx.trait_rename_specs() {
        if !ctx.trait_rename_matched(spec.span) {
            emit_warning_for_span(
                spec.span,
                format!(
                    "autozng: `trait_path = \"{}\"` matched no trait method emitted on its type",
                    spec.trait_path
                ),
            );
        }
    }

    ZngurSpec {
        types,
        funcs,
        traits,
        ..ZngurSpec::default()
    }
}

/// Build one recorded trait into its `ZngurSpec::traits` entry: the [`RustTrait`] key plus the
/// [`ZngurTrait`] block (its associated functions as IDL methods).
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
/// (`emit_items(emit_traits = true)`) carries its concrete type arguments.
fn rust_trait_for(record: &ExportedTraitRecord) -> Option<RustTrait> {
    let path = trait_path(record.decl.def_id);
    match fn_trait_name(&path) {
        Some(name) => Some(fn_trait_rust_trait(record, name)),
        None => {
            let generics = match &record.concrete {
                Some(trait_ref) => concrete_trait_generics(trait_ref)?,
                // A declaration describes a concrete dyn compatible trait, so it has no type or
                // const parameters; lifetimes do not contribute IDL generics.
                None => Vec::new(),
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
/// Returns `None` when an argument cannot be mapped to a concrete IDL type (the whole trait is
/// then dropped by [`rust_trait_for`]).
fn concrete_trait_generics(trait_ref: &rustc_public::ty::TraitRef) -> Option<Vec<RustType>> {
    let mut generics = Vec::new();
    for arg in trait_ref.args().0.iter().skip(1) {
        match arg {
            // Lifetimes are dropped (rustc re-infers them); a const argument has no IDL
            // representation, so the whole trait is dropped.
            GenericArgKind::Lifetime(_) => {}
            GenericArgKind::Type(ty) => {
                generics.push(map_rustc_public_ty_to_zngur(*ty).ok()?);
            }
            GenericArgKind::Const(_) => return None,
        }
    }
    Some(generics)
}

/// The trait's full path as IDL path segments: anything this crate can name from its root uses
/// the `crate::...` prefix, an external crate keeps its own name (the printer adds the leading `::`).
fn trait_path(def: TraitDef) -> Vec<String> {
    visible_path(&def)
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

/// Build one of a trait declaration's associated functions into an IDL [`ZngurMethod`].
///
/// For a declaration (`concrete == false`) there is no concrete `Self`, so the method's identity
/// signature is used: `Self` (and any other generic parameter) appears as itself, and a method
/// whose signature mentions one is warn-and-skipped (the IDL IR has no type parameters). For a
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
        is_safe: Some(matches!(signature.safety, rustc_public::mir::Safety::Safe)),
    })
}

/// The type parameters a function declares itself, as IDL type variables (`fn foo<T>` -> `[T]`).
///
/// `Generics::params` holds only the function's own parameters (the enclosing trait/impl's are
/// counted by `parent_count`), which is exactly what the IDL turbofish lists.
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
/// (`(A, B)`) and primitives are all valid IDL types. The only exception is the empty tuple
/// (`()`, unit), which zngur specialises and which must never be declared. Fields, variants and
/// methods only exist for ADTs; everything else contributes just its layout and well-known
/// traits.
fn build_type(ctx: &AutozngContext, record: &TyRecord) -> Option<ZngurType> {
    // A generic template (e.g. `GenericExample<T>`) has no IDL representation of its own;
    // only its concrete instantiations are emitted (typically enumerated by an `emit_items`
    // const). A type parameter is expected here, not an unsupported shape, so skip silently
    // rather than warn.
    if !record.is_concrete {
        return None;
    }

    let adt = record.adt;

    let rust_ty = match map_rustc_public_ty_to_zngur(record.ty) {
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

    // The empty tuple (unit) is `RustType::UNIT`, a distinguished value in the IDL IR (the
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

    // A truncated type only names itself: the depth budget did not allow walking its interface.
    // An enum still gets its variant names so C++ can match it; a struct/union gets neither
    // fields nor methods.
    let state = ctx.walk_state(&record.ty);
    let walked = state
        .as_ref()
        .map_or(WalkParts::all(), |state| state.walked);
    let declare_only = state
        .as_ref()
        .is_some_and(|state| state.intent == WalkIntent::Declare);
    let walk_fields = walked.contains(WalkParts::FIELDS);
    let walk_variants = walked.contains(WalkParts::VARIANTS);
    let walk_methods = !declare_only
        && (walked.contains(WalkParts::INHERENT) || walked.contains(WalkParts::TRAIT));
    let by_value = state.as_ref().map_or(true, |state| state.seen_by_value);

    let (fields, variants) = if walk_fields {
        match adt {
            // Enums carry their fields per variant; every other ADT is a single struct/union
            // "variant".
            Some(adt) if adt.kind().is_enum() => (Vec::new(), build_variants(ctx, record, &adt)),
            Some(adt) => (build_fields(ctx, record, &adt), Vec::new()),
            None => (Vec::new(), Vec::new()),
        }
    } else if walk_variants && let Some(adt) = adt.filter(|adt| adt.kind().is_enum()) {
        (Vec::new(), build_variant_names(&adt))
    } else {
        (Vec::new(), Vec::new())
    };

    let mut methods: Vec<RenderedMethod> = if walk_methods {
        build_methods(ctx, record)
    } else {
        Vec::new()
    };
    // `#[autozng::emit_on_implementers]` traits/impls contribute their functions to every type
    // they apply to, as if the functions had been enumerated in an `emit_items` const.
    if walk_methods {
        methods.extend(build_emit_for_methods(ctx, record));
    }
    // A C++ class cannot have a field and a method with the same name, and `zngur` rejects the
    // resulting `.zng`. The field keeps its name (it exposes the value). A colliding inherent
    // method is exposed as `{name}_method`; a colliding trait method as `{name}_{Trait}`, which
    // also separates it from an inherent method with the same name. A numeric suffix handles the
    // case where the generated name is taken as well.
    if !fields.is_empty() && !methods.is_empty() {
        let field_names: FxHashSet<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        // `record.fields` is keyed by `DefId`, but the rendered `fields` (after a field was
        // mapped) are what the collision check must agree with, so match by name.
        let field_defs: FxHashMap<&str, DefId> = record
            .fields
            .values()
            .map(|field| (field.def.name.as_str(), field.def.def_id()))
            .collect();
        // A name used by several methods (an inherent and a trait method) collides even without a
        // field.
        let mut name_counts: FxHashMap<String, usize> = FxHashMap::default();
        for field in &fields {
            *name_counts.entry(field.name.clone()).or_default() += 1;
        }
        for method in &methods {
            *name_counts
                .entry(effective_name(&method.details))
                .or_default() += 1;
        }
        // Inherent methods keep their name; their definitions let a trait-method warning point at
        // the inherent method it collides with.
        let inherent_defs: FxHashMap<String, DefId> = methods
            .iter()
            .filter(|method| method.details.use_path.is_none())
            .map(|method| (effective_name(&method.details), method.def_id))
            .collect();
        // Reserve every current name, so a generated name never takes one that is used.
        let mut used: FxHashSet<String> = name_counts.keys().cloned().collect();
        for method in &mut methods {
            let name = effective_name(&method.details);
            let shares_name = name_counts.get(&name).is_some_and(|count| *count > 1);
            let collides_with_field = field_names.contains(name.as_str());
            if !collides_with_field && !(method.details.use_path.is_some() && shares_name) {
                continue;
            }
            let base = match method
                .details
                .use_path
                .as_ref()
                .and_then(|path| path.last())
            {
                Some(trait_name) => format!("{name}_{trait_name}"),
                None => format!("{name}_method"),
            };
            let mut candidate = base.clone();
            let mut suffix = 2;
            while used.contains(&candidate) {
                candidate = format!("{base}{suffix}");
                suffix += 1;
            }
            used.insert(candidate.clone());
            let (other, kind) = if collides_with_field {
                (
                    field_defs
                        .get(name.as_str())
                        .copied()
                        .map(|id| (id, "the field is declared here")),
                    "a field",
                )
            } else {
                (
                    inherent_defs
                        .get(&name)
                        .copied()
                        .map(|id| (id, "the inherent method is declared here")),
                    "another method",
                )
            };
            warn_cpp_name_collision(
                method.def_id,
                method.emit_span,
                other,
                format!(
                    "autozng: method `{}` on `{}` collides with {kind} in C++; exposing it as \
                     `{candidate}`",
                    method.details.data.name, record.ty
                ),
            );
            method.details.cpp_name = Some(candidate);
        }
    }

    // An explicit `#[autozng::layout(...)]` replaces the automatic `StackAllocated` policy
    // derived from the type's real size and alignment.
    let layout = match record.adt.and_then(|adt| ctx.layout_override_for(&adt)) {
        Some(over) => Some(over.policy),
        None => record.layout.and_then(layout_policy),
    };

    // A struct can be constructed from C++ only when every field is described and neither the
    // Rust type nor the layout policy forbids it. Enums construct per variant, unions have no
    // single active field, and `OnlyByRef` deletes the default constructor.
    let constructor = adt
        .filter(|_| walk_fields)
        .filter(|_| by_value)
        .filter(|adt| adt.kind().is_struct())
        // A struct's `#[non_exhaustive]` is on its field list, not on a variant list; the struct's
        // sole variant records it.
        .filter(|adt| {
            adt.variants()
                .into_iter()
                .next()
                .is_none_or(|variant| variant.is_exhaustive())
        })
        .filter(|_| !matches!(&layout, Some(LayoutPolicy::OnlyByRef)))
        .and_then(|adt| {
            let all_fields = adt
                .variants()
                .into_iter()
                .next()
                .map(|variant| variant.fields())
                .unwrap_or_default();
            if fields.is_empty() || fields.len() != all_fields.len() {
                return None;
            }
            Some(ZngurConstructor {
                inputs: fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
            })
        });

    // An `AsGenericArg` declaration is a pure layout stub: it does not search for well-known
    // trait implementations either.
    let mut wellknown = wellknown_traits(record.ty);
    if declare_only
        && state
            .as_ref()
            .is_some_and(|state| state.strongest_reason == VisitReason::AsGenericArg)
    {
        wellknown.clear();
    }

    // The definition tracking was only needed for the collision diagnostic above.
    let methods = methods.into_iter().map(|method| method.details).collect();

    Some(ZngurType {
        ty: rust_ty,
        layout,
        wellknown_traits: wellknown,
        // Only ADTs can be `#[non_exhaustive]`; every other type is exhaustive.
        exhaustive: adt.is_none_or(|adt| adt.is_exhaustive()),
        methods,
        // TODO(render): the well-known-trait data payloads (pretty_print/debug_print/
        // drop_in_place) still need analysis work.
        constructor,
        variants,
        fields,
        cpp_heap_allocated: None,
        cpp_ref: None,
        cpp_stack_owned: None,
    })
}

/// The variant names of an enum truncated at the depth cap.
///
/// The fields and methods are not walked, so every variant is non-exhaustive: C++ can match it
/// but cannot access or construct its fields.
fn build_variant_names(adt: &AdtDef) -> Vec<ZngurVariant> {
    adt.variants()
        .into_iter()
        .filter(|variant| !variant.is_unstable())
        .map(|variant| ZngurVariant {
            name: variant.name().to_string(),
            exhaustive: false,
            fields: Vec::new(),
        })
        .collect()
}

/// The field's type instantiated with the concrete arguments of `record.ty`.
///
/// A generic ADT declares its fields in terms of its type parameters
/// (`Option<T>::Some(T)`); the concrete instance (`Option<Ipv4Addr>`) needs the substitution
/// before the field can be mapped to the IDL.
fn concrete_field_ty(record: &TyRecord, field: &FieldDef) -> Ty {
    let Some(args) = concrete_ty_args(record) else {
        return field.ty();
    };
    with_ctxt(|driver| {
        let tcx = driver.tcx;
        stable(
            tcx.type_of(internal(tcx, field.def_id()))
                .instantiate(tcx, internal(tcx, &args))
                .skip_norm_wip(),
        )
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
            let emit_span = ctx
                .field_emit_span(adt, &field)
                .or_else(|| record.fields.get(&field.def_id()).map(|field| field.span))?;
            let offset = offsets
                .as_ref()
                .and_then(|offsets| offsets.get(index).copied());
            build_field(ctx, record, adt, &field, emit_span, offset)
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
        .filter(|variant| !variant.is_unstable())
        .map(|variant| {
            let all_fields = variant.fields();
            let fields = all_fields
                .iter()
                .filter_map(|field| {
                    let emit_span = ctx
                        .field_emit_span(adt, field)
                        .or_else(|| record.fields.get(&field.def_id()).map(|field| field.span))?;
                    build_field(ctx, record, adt, field, emit_span, None)
                })
                .collect::<Vec<_>>();
            ZngurVariant {
                name: variant.name().to_string(),
                // The IDL's `non_exhaustive` means the field list may be incomplete. A generic
                // instance whose field types were not substituted, or a policy walk that did
                // not record the fields, cannot be constructed from C++.
                exhaustive: variant.is_exhaustive() && fields.len() == all_fields.len(),
                fields,
            }
        })
        .collect()
}

/// Map one selected field to the IDL IR, warning (and returning `None`) when its type cannot
/// be represented so only that field is dropped.
fn build_field(
    ctx: &AutozngContext,
    record: &TyRecord,
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
    let field_ty = concrete_field_ty(record, field);
    match map_rustc_public_ty_to_zngur(field_ty) {
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
    ty.implemented_wellknown_traits()
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
/// The IDL syntax only renames associated methods, through `ZngurMethodDetails::cpp_name`
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

/// The export name recorded for one rendered instantiation of `def` (`#[autozng::rename]` on a
/// `let` binding inside an `emit_items` const).
///
/// The name is keyed by the fn's `DefId` plus the *rendered* generic args (the impl's args from
/// the concrete instance and the method's own enumerated args), so a rename recorded for one
/// instantiation does not leak to another. An empty arg list is the plain `DefId` key — the
/// method declaration's `rename`, which applies to every instantiation.
fn monomorphization_export_name(
    ctx: &AutozngContext,
    def: FnDef,
    args: Option<&GenericArgs>,
) -> Option<ExportName> {
    let args = args.filter(|args| !args.0.is_empty())?;
    ctx.export_name_for(&MetadataKey::for_monomorphization(def.def_id(), args))
}

/// The C++ name a rendered method is exposed under.
fn effective_name(method: &ZngurMethodDetails) -> String {
    method
        .cpp_name
        .clone()
        .unwrap_or_else(|| method.data.name.clone())
}

/// A rendered method plus the definition it came from.
///
/// The definition is not part of the IDL. It is kept until [`build_type`] finishes so a C++ name
/// collision can point its warning at the method through the live `TyCtxt`; [`build_type`] strips
/// it before building the `ZngurType`.
struct RenderedMethod {
    details: ZngurMethodDetails,
    def_id: DefId,
    /// The `#[autozng::emit]` that selected the method, for an `emitted here` note.
    emit_span: Span,
}

/// The C++ name a `rename(trait_path = "...")` spec recorded on the rendered type gives
/// `method_name`, if any.
///
/// The selector is compared to the trait reference exactly as the `.zng` renders it
/// (`::std::convert::From<u32>`, `crate::Trait<::std::string::String>`); only whitespace is
/// ignored. The last matching spec wins.
fn trait_rename_for(
    ctx: &AutozngContext,
    record: &TyRecord,
    trait_fn: &TraitFnRecord,
    method_name: &str,
) -> Option<String> {
    let adt = record.adt?;
    let trait_ref = trait_fn.trait_ref.as_ref()?;
    let rendered = strip_whitespace(&rendered_trait_ref(trait_fn.trait_def, trait_ref)?);
    for spec in ctx.trait_renames_for_adt(&adt).into_iter().rev() {
        if strip_whitespace(&spec.trait_path) == rendered
            && spec
                .method
                .as_deref()
                .is_none_or(|method| method == method_name)
        {
            ctx.mark_trait_rename_matched(spec.span);
            return Some(
                spec.actions
                    .iter()
                    .fold(method_name.to_owned(), |name, action| action.apply(&name)),
            );
        }
    }
    None
}

/// Render a trait reference the way the IDL printer does, for exact selector matching.
fn rendered_trait_ref(trait_def: TraitDef, trait_ref: &TraitRef) -> Option<String> {
    let path = RustPathAndGenerics {
        path: visible_path(&trait_def),
        generics: concrete_trait_generics(trait_ref)?,
        named_generics: Vec::new(),
    };
    let mut buffer = Vec::new();
    {
        let mut printer = IDLPrinter::new(&mut buffer);
        printer.write_idl(&path).ok()?;
    }
    String::from_utf8(buffer).ok()
}

/// Drop all whitespace, so `Trait< A , B >` and `Trait<A, B>` compare equal.
fn strip_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Build the methods recorded for a type (from its inherent impl blocks).
///
/// For a fully monomorphized generic type (e.g. `Container<i32>`) the methods live on the ADT
/// *definition*'s record: the analysis pass walks an ADT's inherent impls once, onto the
/// generic self type. They are looked up there and instantiated with the concrete arguments of
/// `record.ty` (see [`build_method`]).
fn build_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<RenderedMethod> {
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

    // Group the recorded methods by their definition: a method recorded by an `emit_items`
    // binding carries that binding's generic args (`partial_args`), while the impl walk records
    // the uninstantiated template (`partial_args = None`). For one rendered instance, the record
    // whose impl arguments match the instance replaces the template. A record for another
    // instantiation is not rendered here: it would duplicate the method and could apply its
    // rename to the wrong type.
    let mut by_def: IndexMap<FnDef, Vec<(&ImplRecord, &FnRecord)>> = IndexMap::new();
    for impl_record in impls {
        for method in impl_record.methods.iter() {
            by_def
                .entry(method.def)
                .or_default()
                .push((impl_record, method));
        }
    }

    let mut methods: Vec<RenderedMethod> = Vec::new();
    for (def, records) in by_def {
        let parent_count = def.generics_of().parent_count;
        // Whether `method` was recorded for the instance being rendered. A record whose
        // `partial_args` do not cover the impl's parameters is a template for a different
        // instantiation.
        let matches_instance = |method: &FnRecord| -> bool {
            let Some(partial) = method.partial_args.as_ref() else {
                return false;
            };
            if parent_count == 0 {
                return true;
            }
            let Some(instance) = args.as_ref() else {
                return false;
            };
            partial.0.len() >= parent_count
                && instance.0.len() >= parent_count
                && partial.0[..parent_count] == instance.0[..parent_count]
        };
        let has_matching = records.iter().any(|(_, method)| matches_instance(method));
        let has_template = records
            .iter()
            .any(|(_, method)| method.partial_args.is_none());
        let mut selected: Vec<(&ImplRecord, &FnRecord)> = records
            .iter()
            .copied()
            .filter(|(_, method)| {
                if method.partial_args.is_some() {
                    matches_instance(method)
                } else {
                    // The template renders only when no record was recorded for this instance.
                    !has_matching
                }
            })
            .collect();
        // A type reached only through an `emit_items` binding may have no template record; use
        // the first record as a signature template (its per-instance rename will not match).
        if selected.is_empty() && !has_template {
            selected.extend(records.first().copied());
        }
        for (impl_record, method) in selected {
            if let Some(details) = build_method(ctx, record, impl_record, method, args.as_ref()) {
                methods.push(RenderedMethod {
                    details,
                    def_id: method.def.def_id(),
                    emit_span: method.span,
                });
            }
        }
    }

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
/// IDL representation (a type parameter, a const generic, a projection) are dropped.
fn own_generic_args(fn_def: FnDef, args: Option<&GenericArgs>) -> Vec<RustType> {
    let Some(args) = args else {
        return Vec::new();
    };
    args.0
        .iter()
        .skip(fn_def.generics_of().parent_count)
        .filter_map(|arg| match arg {
            GenericArgKind::Type(ty) => map_rustc_public_ty_to_zngur(*ty).ok(),
            _ => None,
        })
        .collect()
}

fn build_method(
    ctx: &AutozngContext,
    record: &TyRecord,
    impl_record: &ImplRecord,
    method: &FnRecord,
    instance_args: Option<&GenericArgs>,
) -> Option<ZngurMethodDetails> {
    let def = method.def;
    if !ctx
        .walk_state(&record.ty)
        .map_or(true, |state| state.seen_by_value)
        && receiver_kind(&def) == ReceiverKind::Value
    {
        return None;
    }
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };
    // The IDL keeps the original Rust name (needed to construct the bindings); a `rename` goes
    // into `cpp_name` as the `fn ... as <name>` suffix.
    let name = name.to_string();
    // The concrete arguments to instantiate `def` with: the method's own explicitly-enumerated
    // args (when an `emit_items` const named it) with the impl's parameters taken from the
    // concrete instance. `None` when they do not cover every generic parameter of the method, in
    // which case its identity signature is rendered and skipped as an unsupported parameter.
    let combined_args = combine_method_args(&def, method.partial_args.as_ref(), instance_args);
    let args = combined_args.as_ref();
    // A `rename` on the method declaration is keyed by the `AssocItem` and applies to every
    // instantiation; one on a `let` binding is keyed by the *rendered* instantiation (its impl
    // args included), so it applies only to that one.
    let cpp_name = monomorphization_export_name(ctx, def, args)
        .or_else(|| ctx.export_name_for(&assoc_item))
        .map(|export| export.name);
    let self_ty = record.ty;
    // The ADT definition's (identity) self type, e.g. `Generic<T>` for a `Generic<i32>` record.
    // Used to still recognize the receiver of a method whose own generics prevent instantiation.
    let self_def_ty = record.adt.map(|adt| adt.ty().base_ty());

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
            is_safe: Some(matches!(signature.safety, rustc_public::mir::Safety::Safe)),
        },
        use_path: None,
        deref: None,
        cpp_name,
    })
}

/// The receiver, inputs and output of an instantiated method/function signature.
///
/// Classifies the receiver from the first parameter; receivers like `Box<Self>`, `Rc<Self>`,
/// `Pin<&Self>`, ... are not representable in the IDL IR yet, so those items are warned
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
            _ => {
                let receiver = first_input
                    .map(|ty| ty.to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                warn(
                    def.def_id(),
                    emit_span,
                    format!("autozng: skipping method `{name}`: unsupported receiver `{receiver}`"),
                );
                return None;
            }
        }
    } else {
        ZngurMethodReceiver::Static
    };

    let inputs = signature
        .inputs()
        .iter()
        .skip(usize::from(has_self))
        .map(|input| map_rustc_public_ty_to_zngur(*input))
        .collect::<Result<Vec<_>, _>>();
    let (inputs, output) = match (inputs, map_rustc_public_ty_to_zngur(signature.output())) {
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
fn build_trait_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<RenderedMethod> {
    record
        .trait_fns
        .iter()
        .filter_map(|trait_fn| {
            build_trait_method(ctx, record, trait_fn).map(|details| RenderedMethod {
                details,
                def_id: trait_fn.def.def_id(),
                emit_span: trait_fn.span,
            })
        })
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
fn build_emit_for_methods(ctx: &AutozngContext, record: &TyRecord) -> Vec<RenderedMethod> {
    let mut seen: FxHashSet<FnDef> = record
        .trait_fns
        .iter()
        .map(|trait_fn| trait_fn.def)
        .collect();
    let mut methods = Vec::new();
    let type_filter = ctx
        .walk_state(&record.ty)
        .map(|state| state.filter)
        .unwrap_or_else(|| {
            record
                .adt
                .map(|adt| ctx.effective_filter_for_adt(&adt))
                .unwrap_or_default()
        });

    for emit_for in ctx.emit_for_records().iter() {
        let Some(trait_ref) =
            record
                .ty
                .resolve_emit_for(emit_for.trait_def, emit_for.impl_def, &emit_for.clauses)
        else {
            continue;
        };
        let filter = ctx
            .walk_filter_for(&emit_for.trait_def)
            .unwrap_or_else(|| type_filter.clone());
        for trait_fn in emit_for.fns.iter() {
            if !seen.insert(trait_fn.def) {
                continue;
            }
            let Some(assoc_item) = trait_fn.def.associated_item() else {
                continue;
            };
            let AssocKind::Fn { name, .. } = &assoc_item.kind else {
                continue;
            };
            let name = name.to_string();
            let trait_path = emit_for.trait_def.name();
            let item = WalkItem {
                kind: WalkItemKind::TraitMethod,
                name: Some(&name),
                receiver: Some(receiver_kind(&trait_fn.def)),
                visibility: Some(visibility_kind(trait_fn.def.visibility())),
                trait_path: Some(trait_path.trim_start_matches("::")),
            };
            if !filter.allows(&item) {
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
                methods.push(RenderedMethod {
                    details: method,
                    def_id: trait_fn.def.def_id(),
                    emit_span: emit_for.span,
                });
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
    if !ctx
        .walk_state(&record.ty)
        .map_or(true, |state| state.seen_by_value)
        && receiver_kind(&def) == ReceiverKind::Value
    {
        return None;
    }
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };
    let name = name.to_string();
    // See `build_method`: a `rename` on the `let` binding is keyed by the named monomorphization.
    let cpp_name = monomorphization_export_name(ctx, def, Some(&trait_fn.generics))
        .map(|export| export.name)
        .or_else(|| trait_rename_for(ctx, record, trait_fn, &name))
        .or_else(|| ctx.export_name_for(&assoc_item).map(|export| export.name));
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
    let use_path = trait_path(trait_fn.trait_def);

    Some(ZngurMethodDetails {
        data: ZngurMethod {
            name,
            generics: own_generic_args(def, Some(&trait_fn.generics)),
            receiver,
            inputs,
            output,
            is_safe: Some(matches!(signature.safety, rustc_public::mir::Safety::Safe)),
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

    let inputs = signature
        .inputs()
        .iter()
        .map(|input| map_rustc_public_ty_to_zngur(*input))
        .collect::<Result<Vec<_>, _>>();
    let (inputs, output) = match (inputs, map_rustc_public_ty_to_zngur(signature.output())) {
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

    let path = visible_path(&fn_def);

    Some(ZngurFn {
        path: RustPathAndGenerics {
            path,
            generics: own_generic_args(fn_def, record.partial_args.as_ref()),
            named_generics: Vec::new(),
        },
        inputs,
        output,
        is_safe: Some(matches!(signature.safety, rustc_public::mir::Safety::Safe)),
    })
}

/// Print `spec` as an IDL to `path`, creating parent directories as needed.
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
    // A skipped/unsupported item from another crate is not actionable by this crate's user; its
    // warn-and-skip diagnostic is only emitted with the driver's verbose flag.
    if !with_ctxt(|ctx| ctx.cfg.verbose || internal(ctx.tcx, def_id).is_local()) {
        return;
    }
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

/// Warn that a method's C++ name collides with another member: the method definition is the
/// primary span, with the other member's definition as a note.
///
/// Only valid while the analysis callback is running (it needs the live `TyCtxt`).
fn warn_cpp_name_collision(
    def_id: DefId,
    emit_span: Span,
    other: Option<(DefId, &'static str)>,
    msg: impl Into<DiagMessage>,
) {
    let item_span = span_of_def(def_id);
    with_struct_warning_for_rustc_public_span(def_id.span(), msg, |mut diag, _tcx| {
        if let Some((other_id, label)) = other {
            let other_span = span_of_def(other_id);
            if !other_span.is_dummy() && other_span != item_span {
                diag.span_note(other_span, label);
            }
        }
        if !emit_span.is_dummy() && !item_span.contains(emit_span) {
            diag.span_note(emit_span, "emitted here");
        }
        diag.emit()
    });
}
