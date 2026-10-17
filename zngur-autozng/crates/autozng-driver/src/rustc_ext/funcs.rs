use rustc_public::{
    CrateDef,
    rustc_internal::{internal, stable},
};

use crate::driver::with_ctxt;

pub(crate) trait GetFnInfoExt {
    fn opt_fn_sig(&self) -> Option<rustc_public::ty::FnSig>;
    fn opt_fn_def(&self) -> Option<rustc_public::ty::FnDef>;
}

impl GetFnInfoExt for rustc_public::ty::AssocItem {
    fn opt_fn_sig(&self) -> Option<rustc_public::ty::FnSig> {
        with_ctxt(|ctx| match &self.kind {
            rustc_public::ty::AssocKind::Fn { .. } => {
                let rustc_def_id = internal(ctx.tcx, self.def_id.def_id());
                Some(stable(
                    ctx.tcx
                        .fn_sig(rustc_def_id)
                        .instantiate_identity()
                        .skip_normalization()
                        .skip_binder(),
                ))
            }
            _ => None,
        })
    }
    fn opt_fn_def(&self) -> Option<rustc_public::ty::FnDef> {
        match &self.kind {
            rustc_public::ty::AssocKind::Fn { .. } => {
                Some(rustc_public::ty::FnDef(self.def_id.def_id()))
            }
            _ => None,
        }
    }
}
