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

use rustc_errors::DiagMessage;
use rustc_public::ty::{AdtDef, AssocKind, RigidTy, Ty, TyKind};
use rustc_public::{CrateDef, DefId};
use zngur_def::printing::IDLPrinter;
use zngur_def::{
    LayoutPolicy, RustType, ZngurMethod, ZngurMethodDetails, ZngurMethodReceiver, ZngurSpec,
    ZngurType, ZngurWellknownTrait,
};

use crate::ctx::{AutozngContext, FnRecord, TyRecord};
use crate::driver::with_ctxt;
use crate::error::emit_warning_for_rustc_public_span;
use crate::rustc_ext::{AdtDefExt, WellKownTraitsExt, WellKnownTrait};
use crate::type_map::map_rustc_public_ty_to_zngur;

/// Build the [`ZngurSpec`] from everything the analysis pass recorded into `ctx`.
///
/// Must be called inside the `after_analysis` callback (the context borrows the `TyCtxt`).
///
/// Currently only type-level information is emitted: the type path, layout, well-known
/// traits and non-exhaustiveness, plus any methods recorded in [`TyRecord::impls`]. Fields,
/// variants, constructors and trait emission depend on analysis work that is not wired up
/// yet.
pub fn build_spec(ctx: &AutozngContext) -> ZngurSpec {
    let mut types: Vec<ZngurType> = ctx
        .ty_records()
        .values()
        .filter_map(|record| build_type(ctx, record))
        .collect();

    // `ty_records` is a hash map; sort for stable output.
    types.sort_by_cached_key(|ty| ty.ty.to_string());

    ZngurSpec {
        types,
        ..ZngurSpec::default()
    }
}

/// Convert a single recorded type into a [`ZngurType`], or `None` if it cannot be represented
/// (with a warning emitted).
fn build_type(ctx: &AutozngContext, record: &TyRecord) -> Option<ZngurType> {
    // Only ADTs have a meaningful `type ... { .. }` representation. Primitive/aggregate
    // records (from `record_ty` dependencies) are emitted as part of their users instead.
    let adt = record.adt?;

    let rust_ty = match with_ctxt(|driver| map_rustc_public_ty_to_zngur(record.ty, driver.tcx)) {
        Ok(ty) => ty,
        Err(msg) => {
            warn(adt.def_id(), format!("autozng: skipping `{}`: {msg}", record.ty));
            return None;
        }
    };
    let rust_ty = apply_export_name(ctx, &adt, rust_ty);

    Some(ZngurType {
        ty: rust_ty,
        layout: record.layout.and_then(layout_policy),
        wellknown_traits: wellknown_traits(record.ty),
        exhaustive: adt.is_exhaustive(),
        methods: build_methods(record),
        // TODO(render): fields/variants/constructor and the well-known-trait data payloads
        // (pretty_print/debug_print/drop_in_place) require the analysis pass to record
        // per-field/variant information and to call `record_impl_method`.
        constructor: None,
        variants: Vec::new(),
        fields: Vec::new(),
        cpp_heap_allocated: None,
        cpp_ref: None,
        cpp_stack_owned: None,
    })
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

/// Apply a recorded `#[autozng::rename]` to an ADT type.
///
/// `rename` only changes the item's own name, so just the last path segment is replaced; the
/// crate/module path produced by the type mapper is left intact for discoverability.
fn apply_export_name(ctx: &AutozngContext, adt: &AdtDef, rust_ty: RustType) -> RustType {
    let Some(export_name) = ctx.export_name_for(adt) else {
        return rust_ty;
    };
    match rust_ty {
        RustType::Adt(mut path) => {
            if let Some(last) = path.path.last_mut() {
                *last = export_name;
            }
            RustType::Adt(path)
        }
        other => other,
    }
}

/// Build the methods recorded for a type (currently always empty; see
/// [`AutozngContext::record_impl_method`]).
fn build_methods(record: &TyRecord) -> Vec<ZngurMethodDetails> {
    record
        .impls
        .values()
        .flat_map(|impl_record| impl_record.methods.iter())
        .filter_map(|method| build_method(record, method))
        .collect()
}

fn build_method(record: &TyRecord, method: &FnRecord) -> Option<ZngurMethodDetails> {
    let def = method.def;
    let assoc_item = def.associated_item()?;
    let AssocKind::Fn { name, has_self } = &assoc_item.kind else {
        return None;
    };

    let signature = def.fn_sig().skip_binder();
    let self_ty = record.ty;

    // Classify the receiver from the first parameter. Receivers like `Box<Self>`, `Rc<Self>`,
    // `Pin<&Self>`, ... are not representable in the `.zng` IR yet, so those methods are
    // warned about and skipped.
    let receiver = if *has_self {
        match signature.inputs().first().map(|ty| ty.kind()) {
            Some(TyKind::RigidTy(RigidTy::Ref(_, inner, mutability))) if inner == self_ty => {
                ZngurMethodReceiver::Ref(match mutability {
                    rustc_public::mir::Mutability::Mut => zngur_def::Mutability::Mut,
                    rustc_public::mir::Mutability::Not => zngur_def::Mutability::Not,
                })
            }
            Some(TyKind::RigidTy(RigidTy::Adt(adt, _))) if adt.ty() == self_ty => {
                ZngurMethodReceiver::Move
            }
            other => {
                warn(
                    def.def_id(),
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
            .skip(usize::from(*has_self))
            .map(|input| map_rustc_public_ty_to_zngur(*input, driver.tcx))
            .collect::<Result<Vec<_>, _>>()
    });
    let inputs = match inputs {
        Ok(inputs) => inputs,
        Err(msg) => {
            warn(def.def_id(), format!("autozng: skipping method `{name}`: {msg}"));
            return None;
        }
    };

    let output =
        match with_ctxt(|driver| map_rustc_public_ty_to_zngur(signature.output(), driver.tcx)) {
            Ok(output) => output,
            Err(msg) => {
                warn(def.def_id(), format!("autozng: skipping method `{name}`: {msg}"));
                return None;
            }
        };

    Some(ZngurMethodDetails {
        data: ZngurMethod {
            name: name.to_string(),
            // TODO(render): generic methods (`partial_args`) are not handled yet.
            generics: Vec::new(),
            receiver,
            inputs,
            output,
            is_safe: matches!(signature.safety, rustc_public::mir::Safety::Safe),
        },
        use_path: None,
        deref: None,
        cpp_name: None,
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

/// Emit a warning attached to `def_id`'s span.
///
/// The `DefId` should be the type/ADT (`TyRecord::adt`, falling back to `TyRecord::ty`'s
/// `adt_def`) or the exact function definition / impl block the warning is about, so the
/// diagnostic points at the offending source location.
///
/// Only valid while the analysis callback is running (it needs the live `TyCtxt`).
fn warn(def_id: DefId, msg: impl Into<DiagMessage>) {
    emit_warning_for_rustc_public_span(def_id.span(), msg);
}
