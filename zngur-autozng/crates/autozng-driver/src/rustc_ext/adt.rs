use rustc_hir::def::DefKind;
use rustc_public::{
    CrateDef,
    rustc_internal::{internal, stable},
    ty::{AdtDef, FieldDef, VariantDef},
};

use crate::driver::with_ctxt;

/// Extensions for [`rustc_public::ty::AdtDef`] that `rustc_public` does not expose.
pub trait AdtDefExt {
    /// `true` when the ADT's variant list is *not* `#[non_exhaustive]`.
    fn is_exhaustive(&self) -> bool;
}

impl AdtDefExt for AdtDef {
    fn is_exhaustive(&self) -> bool {
        with_ctxt(|driver| {
            let def_id = internal(driver.tcx, self.def_id());
            !driver.tcx.adt_def(def_id).is_variant_list_non_exhaustive()
        })
    }
}

/// Extensions for [`rustc_public::ty::FieldDef`] that `rustc_public` does not expose.
pub(crate) trait FieldDefExt {
    /// The ADT that owns this field, walking up through enum variants.
    ///
    /// A struct/union field's parent is directly the ADT; an enum field's parent is its
    /// variant, whose own parent is the enum.
    fn adt_def(&self) -> Option<AdtDef>;
}

impl FieldDefExt for FieldDef {
    fn adt_def(&self) -> Option<AdtDef> {
        with_ctxt(|driver| {
            let tcx = driver.tcx;
            let mut current = internal(tcx, self.def_id());
            loop {
                let parent = tcx.opt_parent(current)?;
                match tcx.def_kind(parent) {
                    DefKind::Struct | DefKind::Union | DefKind::Enum => {
                        return Some(AdtDef(stable(parent)));
                    }
                    DefKind::Variant => current = parent,
                    _ => return None,
                }
            }
        })
    }
}

/// Extensions for [`rustc_public::ty::VariantDef`] that `rustc_public` does not expose.
pub(crate) trait VariantDefExt {
    /// `true` when this variant's field list is *not* `#[non_exhaustive]`.
    fn is_exhaustive(&self) -> bool;
}

impl VariantDefExt for VariantDef {
    fn is_exhaustive(&self) -> bool {
        let adt = self.adt_def();
        // `rustc_public`'s `VariantDef` does not expose its `DefId`. Its variants are in index
        // order, so its position in the ADT identifies the matching rustc variant.
        let Some(index) = adt.variants().iter().position(|variant| variant == self) else {
            return true;
        };
        with_ctxt(|driver| {
            let tcx = driver.tcx;
            tcx.adt_def(internal(tcx, adt.def_id()))
                .variants()
                .into_iter()
                .nth(index)
                .is_none_or(|variant| !variant.is_field_list_non_exhaustive())
        })
    }
}
