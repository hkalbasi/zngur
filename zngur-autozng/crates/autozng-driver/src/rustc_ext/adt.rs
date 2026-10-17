use rustc_public::{CrateDef, rustc_internal::internal, ty::AdtDef};

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
