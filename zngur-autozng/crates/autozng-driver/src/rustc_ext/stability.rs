//! Stability gating for items that cross the generated bridge.
//!
//! The bridge is currently compiled with the stable toolchain. It could be built with a nightly
//! compiler, but there is no reliable way to know which unstable features exposed by the driver's
//! pinned nightly are also available in the nightly the target crate uses. Supporting unstable
//! items for export is therefore not attempted; they are skipped silently, since they are
//! incidental to the interface (an external type's inherent impl, a policy walk, a re-export)
//! rather than explicitly selected.

use rustc_public::CrateDef;
use rustc_public::rustc_internal::internal;

use crate::driver::with_ctxt;

pub(crate) trait StabilityExt {
    /// Whether the definition is `#[unstable]` on the driver's toolchain.
    fn is_unstable(&self) -> bool;
}

impl<T: CrateDef> StabilityExt for T {
    fn is_unstable(&self) -> bool {
        is_unstable_def_id(self.def_id())
    }
}

// `DefId` cannot have its own `StabilityExt` impl next to the blanket one: coherence has to
// assume a future `CrateDef for DefId`.
fn is_unstable_def_id(def_id: rustc_public::DefId) -> bool {
    with_ctxt(|ctx| {
        ctx.tcx
            .lookup_stability(internal(ctx.tcx, def_id))
            .is_some_and(|stability| stability.is_unstable())
    })
}
