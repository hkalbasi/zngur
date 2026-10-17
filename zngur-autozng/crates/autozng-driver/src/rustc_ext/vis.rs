use rustc_middle::ty::TyCtxt;
use rustc_public::{CrateDef, rustc_internal::internal};
use rustc_span::def_id::{CRATE_DEF_ID, DefId as RustcDefId};

use crate::driver::with_ctxt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Visibility {
    Public,
    PublicInternal,
    PrivateAPI,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "pub"),
            Self::PublicInternal => write!(f, "pub(crate)"),
            Self::PrivateAPI => write!(f, ""),
        }
    }
}

pub(crate) trait VisibilityExt {
    fn visibility(&self) -> Visibility;
}

/// Classify a definition's visibility for the `.zng` interface.
///
/// Zngur's generated code lives in the crate root, so an item is usable only when it is visible
/// from there (or from outside the crate). Three cases:
///
/// - [`Visibility::Public`] — the item and its whole module chain are `pub`, i.e. it is part of
///   the crate's public interface. Emitted by default.
/// - [`Visibility::PublicInternal`] — the item is visible from the crate root (`pub(crate)`,
///   `pub(super)` resolving to the root, or private at the crate root), but not outside. Fine to
///   emit, but only when explicitly tagged `emit` / reached through an emitted item's interface.
/// - [`Visibility::PrivateAPI`] — the item is **not** visible from the crate root (a private item
///   in a nested module, or a `pub` item whose module chain is not all `pub`). The generated code
///   cannot see it, so it must not be emitted.
fn visibility(tcx: TyCtxt, id: RustcDefId) -> Visibility {
    let Some(local_id) = id.as_local() else {
        // An external item is part of another crate's public interface.
        return Visibility::Public;
    };
    let vis = tcx.visibility(local_id);

    // The generated code lives at the crate root, so an item the root cannot see must not be
    // emitted. `Visibility::is_accessible_from` answers exactly that for the item's own
    // visibility.
    if !vis.is_accessible_from(CRATE_DEF_ID, tcx) {
        return Visibility::PrivateAPI;
    }

    // A `pub` item is part of the crate's public interface only when every enclosing module is
    // `pub` too: a `pub` item behind a non-`pub` module is not reachable from the crate root's
    // public interface, so it is treated as private.
    if vis.is_public() {
        if enclosing_modules_are_all_public(tcx, local_id) {
            Visibility::Public
        } else {
            Visibility::PrivateAPI
        }
    } else {
        // `pub(crate)` / `pub(super)` resolving to the root / private at the crate root: visible
        // from the crate root, but not outside.
        Visibility::PublicInternal
    }
}

/// Whether every module enclosing `def_id` (up to the crate root) is declared `pub`.
///
/// An item's own `pub` only reaches outside the crate when the whole module chain is `pub`; a
/// non-`pub` module in between hides it from the crate root's public interface.
fn enclosing_modules_are_all_public(
    tcx: TyCtxt<'_>,
    def_id: rustc_span::def_id::LocalDefId,
) -> bool {
    let mut module = tcx.parent_module_from_def_id(def_id);
    loop {
        if module.is_top_level_module() {
            return true;
        }
        if !tcx.visibility(module).expect_local().is_public() {
            return false;
        }
        module = tcx.parent_module_from_def_id(module.into());
    }
}

impl VisibilityExt for rustc_public::ty::AdtDef {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id())))
    }
}

impl VisibilityExt for rustc_public::ty::TraitDef {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id())))
    }
}

impl VisibilityExt for rustc_public::ty::AssocItem {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id.def_id())))
    }
}

impl VisibilityExt for rustc_public::ty::FnDef {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id())))
    }
}

impl VisibilityExt for rustc_public::ty::FieldDef {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id())))
    }
}
