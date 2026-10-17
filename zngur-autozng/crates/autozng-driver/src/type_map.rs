//! Bridge from `rustc_public` types to `zngur_def` IR types.
//!
//! [`map_rustc_public_ty_to_zngur`] recursively converts a `Ty` into a
//! [`zngur_def::RustType`]. A fully monomorphized generic type maps successfully as long as
//! every generic argument is either a lifetime (dropped, rustc re-infers it) or a concrete
//! [`RigidTy`]; the concrete arguments are rendered as the `.zng` type parameters.
//!
//! Unsupported shapes (arrays, fn pointers, closures, const generics, opaque params, ...)
//! return [`MapTypeError::Unsupported`] so callers can warn at the offending span and skip
//! only that item, preserving the rest of output. A type that still mentions an unresolved
//! generic projection / higher-ranked variable ([`TyKind::Alias`] / [`TyKind::Bound`]) returns
//! [`MapTypeError::SkipSilently`]: it is skipped without a diagnostic.

use rustc_middle::ty::TyCtxt;
use rustc_public::{
    CrateDef,
    mir::Mutability,
    ty::{
        Binder, BoundTyKind, BoundVariableKind, ExistentialPredicate, FloatTy, GenericArgKind,
        IntTy, RigidTy, TermKind, Ty, TyKind, UintTy,
    },
};
use zngur_def::{PrimitiveRustType, RustPathAndGenerics, RustTrait, RustType};

const PATH_SEP: &str = "::";

/// Why [`map_rustc_public_ty_to_zngur`] could not map a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapTypeError {
    /// The type has no `.zng` representation. Callers warn at the offending span and skip only
    /// the item that mentions it.
    Unsupported(String),
    /// The type still mentions an unresolved generic projection / higher-ranked variable
    /// (`TyKind::Alias` / `TyKind::Bound`). Callers skip the item **without** a warning.
    SkipSilently,
}

impl MapTypeError {
    fn unsupported(msg: impl Into<String>) -> Self {
        MapTypeError::Unsupported(msg.into())
    }
}

impl std::fmt::Display for MapTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapTypeError::Unsupported(msg) => f.write_str(msg),
            MapTypeError::SkipSilently => f.write_str("type cannot be resolved"),
        }
    }
}

pub fn map_rustc_public_ty_to_zngur(ty: Ty, tcx: TyCtxt) -> Result<RustType, MapTypeError> {
    match ty.kind() {
        // An unresolved associated-type projection or a higher-ranked variable has no `.zng`
        // spelling; drop the item silently (the concrete instantiation already happened where
        // possible, see `render::build_method`).
        TyKind::Alias(..) | TyKind::Bound(..) => Err(MapTypeError::SkipSilently),
        TyKind::RigidTy(rty) => match rty {
            RigidTy::Bool => Ok(RustType::Primitive(PrimitiveRustType::Bool)),
            RigidTy::Char => Ok(RustType::Primitive(PrimitiveRustType::Char)),
            RigidTy::Int(int_ty) => Ok(RustType::Primitive(PrimitiveRustType::Int(match int_ty {
                IntTy::I8 => 8,
                IntTy::I16 => 16,
                IntTy::I32 => 32,
                IntTy::I64 => 64,
                IntTy::I128 => 128,
                IntTy::Isize => rustc_public::target::MachineInfo::target()
                    .pointer_width
                    .bits() as u32,
            }))),
            RigidTy::Uint(uint_ty) => match uint_ty {
                UintTy::Usize => Ok(RustType::Primitive(PrimitiveRustType::Usize)),
                _ => Ok(RustType::Primitive(PrimitiveRustType::Uint(
                    match uint_ty {
                        UintTy::U8 => 8,
                        UintTy::U16 => 16,
                        UintTy::U32 => 32,
                        UintTy::U64 => 64,
                        UintTy::U128 => 128,
                        UintTy::Usize => {
                            return Err(MapTypeError::unsupported(
                                "usize is not currently supported",
                            ));
                        }
                    },
                ))),
            },
            RigidTy::Float(float_ty) => Ok(RustType::Primitive(PrimitiveRustType::Float(
                match float_ty {
                    FloatTy::F16 => 16,
                    FloatTy::F32 => 32,
                    FloatTy::F64 => 64,
                    FloatTy::F128 => 128,
                },
            ))),
            RigidTy::Adt(adt_def, generics_args) => {
                let adt_generics = adt_def.generics_of();

                if adt_generics.parent_count > 0 {
                    Err(MapTypeError::unsupported(
                        "types with nested generics are not currently supported in a zngur interface",
                    ))
                } else {
                    let mut path: Vec<String> =
                        adt_def.name().split(PATH_SEP).map(Into::into).collect();
                    if adt_def.krate().is_local && !path.is_empty() {
                        // `rustc_public` reports the local crate's own items under the crate
                        // name, whereas the Zngur IR marks local paths with a leading `crate`
                        // segment (the generator substitutes the real crate name back in).
                        path[0] = "crate".to_string();
                    }

                    let rustc_generic_args =
                        rustc_public::rustc_internal::internal(tcx, &generics_args);
                    let rustc_adt_def = rustc_public::rustc_internal::internal(tcx, adt_def);
                    let rustc_adt_generics = tcx.generics_of(rustc_adt_def.did());

                    let generics = generics_args
                        .0
                        .iter()
                        .zip(rustc_generic_args.iter())
                        .zip(&rustc_adt_generics.own_params)
                        .filter_map(|((arg, rustc_arg), arg_def)| {
                            if arg_def.default_value(tcx).is_some_and(|default_arg| {
                                default_arg.skip_binder() == rustc_arg
                            }) {
                                // arg and it's default are the same (ie. a global allocator on Vec<T >):
                                // don't blindly require the type to be exported
                                None
                            } else {
                                match arg {
                                    GenericArgKind::Lifetime(_) => None, // don't need to preserve lifetimes, re-inferred by rustc
                                    GenericArgKind::Const(_) => Some(Err(MapTypeError::unsupported("types with const generics are not currently supported in a zngur interface"))),
                                    // A fully monomorphized generic is only emitted when every
                                    // non-lifetime argument is a concrete `RigidTy`; an
                                    // unresolved parameter (`Param`/`Alias`/`Bound`) bubbles up
                                    // from the recursive call (warned or skipped silently).
                                    GenericArgKind::Type(param_ty) => Some(map_rustc_public_ty_to_zngur(*param_ty, tcx)),
                                }
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(RustType::Adt(RustPathAndGenerics {
                        path,
                        generics,
                        named_generics: Vec::new(),
                    }))
                }
            }
            RigidTy::Str => Ok(RustType::Primitive(PrimitiveRustType::Str)),
            RigidTy::Slice(slice_ty) => Ok(RustType::Slice(Box::new(
                map_rustc_public_ty_to_zngur(slice_ty, tcx)?,
            ))),
            RigidTy::RawPtr(ptr_ty, mutability) => Ok(RustType::Raw(
                match mutability {
                    Mutability::Mut => zngur_def::Mutability::Mut,
                    Mutability::Not => zngur_def::Mutability::Not,
                },
                Box::new(map_rustc_public_ty_to_zngur(ptr_ty, tcx)?),
            )),
            RigidTy::Ref(_region, ref_ty, mutability) => Ok(RustType::Ref(
                match mutability {
                    Mutability::Mut => zngur_def::Mutability::Mut,
                    Mutability::Not => zngur_def::Mutability::Not,
                },
                Box::new(map_rustc_public_ty_to_zngur(ref_ty, tcx)?),
            )),
            RigidTy::Dynamic(predicates, _region) => {
                let mut pred_iter = predicates.into_iter();
                if let Some(Binder {
                    value: ExistentialPredicate::Trait(trait_ref),
                    bound_vars,
                }) = pred_iter.next()
                {
                    let mut named_generics: Vec<(String, RustType)> = Vec::new();
                    let mut auto_traits = Vec::new();

                    let path: Vec<String> = trait_ref
                        .def_id
                        .name()
                        .as_str()
                        .split(PATH_SEP)
                        .map(Into::into)
                        .collect();
                    let generics = bound_vars
                        .into_iter()
                        .filter_map(|bound_var| match bound_var {
                            BoundVariableKind::Ty(bound_ty) => match bound_ty {
                                BoundTyKind::Anon => Some(Err(MapTypeError::unsupported(
                                    "can not use anonymous bounds in a zngur interface",
                                ))),
                                BoundTyKind::Param(param_def, _) => {
                                    let rustc_def_id = rustc_public::rustc_internal::internal(
                                        tcx,
                                        param_def.def_id(),
                                    );
                                    let rustc_ty = tcx.type_of(rustc_def_id);
                                    let bound_ty_def = rustc_public::rustc_internal::stable(
                                        rustc_ty.skip_binder(),
                                    );
                                    Some(map_rustc_public_ty_to_zngur(bound_ty_def, tcx))
                                }
                            },
                            BoundVariableKind::Region(_) => None, // filter out lifetime bounds
                            BoundVariableKind::Const => Some(Err(MapTypeError::unsupported(
                                "const generics are not supported in zngur interfaces",
                            ))),
                        })
                        .collect::<Result<Vec<_>, _>>()?;

                    for pred in pred_iter {
                        match pred.value {
                            ExistentialPredicate::Projection(projection) => {
                                named_generics.push((
                                    projection
                                        .def_id
                                        .name()
                                        .split(PATH_SEP)
                                        .last()
                                        .unwrap()
                                        .to_string(),
                                    map_rustc_public_ty_to_zngur(match projection.term {
                                        TermKind::Type(term_ty) => Ok(term_ty),
                                        TermKind::Const(_) => Err(MapTypeError::unsupported("const generics are not supported in zngur interfaces"))
                                    }?, tcx)?,
                                ));
                            }
                            ExistentialPredicate::AutoTrait(auto_trait_def) => {
                                // full path to auto trait
                                auto_traits.push(format!("::{}", auto_trait_def.name()));
                            }
                            _ => {
                                // only autotraits and projected terms should be after the first trait def
                                tcx.dcx()
                                    .bug("malformed dynamic object definition with multiple non-auto traits")
                            }
                        }
                    }

                    let rust_trait = match path.iter().map(String::as_str).collect::<Vec<_>>()[..] {
                        ["std", "ops", name @ ("Fn" | "FnMut" | "FnOnce")] => RustTrait::Fn {
                            name: name.to_string(),
                            inputs: generics,
                            output: named_generics
                                .into_iter()
                                .find_map(|(name, out_ty)| {
                                    if name == "Output" {
                                        Some(Box::new(out_ty))
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| {
                                    tcx.dcx()
                                        .bug("Fn() style trait without associated Output type");
                                }),
                        },
                        _ => RustTrait::Normal(RustPathAndGenerics {
                            path,
                            generics,
                            named_generics,
                        }),
                    };

                    Ok(RustType::Dyn(rust_trait, auto_traits))
                } else {
                    tcx.dcx()
                        .bug("malformed dynamic object definition without primary trait");
                }
            }
            RigidTy::Tuple(tys) => Ok(RustType::Tuple(
                tys.into_iter()
                    .map(|ty| map_rustc_public_ty_to_zngur(ty, tcx))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            RigidTy::FnDef(_fn_def, _generics) => Err(MapTypeError::unsupported(
                "function definition types are not supported in a zngur interface",
            )),
            RigidTy::FnPtr(_fn_sig) => Err(MapTypeError::unsupported(
                "function pointer types are not supported in a zngur interface",
            )),
            RigidTy::Closure(_closure_def, _generics) => Err(MapTypeError::unsupported(
                "closure types are not supported in a zngur interface",
            )),
            RigidTy::Never => Err(MapTypeError::unsupported(
                "the `never` type is not supported in a zngur interface",
            )),
            RigidTy::Foreign(_) => Err(MapTypeError::unsupported(
                "foreign types are not supported in a zngur interface",
            )),
            RigidTy::Array(_, _) => Err(MapTypeError::unsupported(
                "array types are not supported in a zngur interface",
            )),
            RigidTy::Pat(_, _) => Err(MapTypeError::unsupported(
                "pattern types are not supported in a zngur interface",
            )),
            RigidTy::Coroutine(_, _)
            | RigidTy::CoroutineClosure(_, _)
            | RigidTy::CoroutineWitness(_, _) => Err(MapTypeError::unsupported(
                "coroutine types are not supported in a zngur interface",
            )),
        },
        TyKind::Param(_ty_param) => Err(MapTypeError::unsupported(
            "type parameters are not supported in a zngur interface",
        )),
    }
}
