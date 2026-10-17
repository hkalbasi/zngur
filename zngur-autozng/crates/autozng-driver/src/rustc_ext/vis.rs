use rustc_middle::ty::TyCtxt;
use rustc_public::{CrateDef, rustc_internal::internal};
use rustc_span::def_id::DefId as RustcDefId;

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

fn visibility(tcx: TyCtxt, id: RustcDefId) -> Visibility {
    if let Some(local_id) = id.as_local() {
        match tcx.visibility(local_id).expect_local() {
            rustc_middle::ty::Visibility::Public => Visibility::Public,
            rustc_middle::ty::Visibility::Restricted(local_id) => {
                if local_id.is_top_level_module() {
                    Visibility::PublicInternal
                } else {
                    Visibility::PrivateAPI
                }
            }
        }
    } else {
        Visibility::Public
    }
}

impl VisibilityExt for rustc_public::ty::AdtDef {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id())))
    }
}

impl VisibilityExt for rustc_public::ty::AssocItem {
    fn visibility(&self) -> Visibility {
        with_ctxt(|ctx| visibility(ctx.tcx, internal(ctx.tcx, self.def_id.def_id())))
    }
}
