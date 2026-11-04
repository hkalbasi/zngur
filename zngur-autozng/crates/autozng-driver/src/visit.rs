//! The two crate-walking visitors plus the `rustc_public` visit trait they share.
//!
//! - [`HirVisitor`] walks the HIR to find `const`/`ty`/`let` items (things the
//!   `rustc_public` API exposes poorly).
//! - [`CrateAnalysisVisitor`](crate::analyze) (defined in `analyze.rs`) walks the local
//!   crate's ADTs, free functions, trait decls and ADT inherent impl blocks via
//!   [`RustcPublicVisitor`].
//!
//! Trait *decls* are visited (`visit_trait`) and their associated functions recorded for an
//! `emit`-tagged trait. Trait *impls* are still stubbed (`ImplDef`'s `super_visit` is a no-op
//! and there is no `visit_impl`). (ADT *inherent* impls are walked directly from `visit_adt`.)

use rustc_middle::ty::TyCtxt;

use std::ops::ControlFlow;

use rustc_public::{
    Crate,
    mir::mono::StaticDef,
    ty::{AdtDef, FnDef, TraitDef},
};

use crate::{
    attrs::{AttrCollectionExt, AutozngAttrsExt},
    ctx::AutozngContext,
    rustc_ext::{HirConstBodyExt, HirItemExt},
};

pub struct HirVisitor<'ctx, 'tcx> {
    pub ctx: &'ctx mut AutozngContext,
    pub tcx: TyCtxt<'tcx>,
}

pub struct OnlyItems();

impl<'tcx> rustc_hir::intravisit::NestedFilter<'tcx> for OnlyItems {
    type MaybeTyCtxt = TyCtxt<'tcx>;

    const INTER: bool = true;
    const INTRA: bool = false;
}

impl<'tcx> rustc_hir::intravisit::Visitor<'tcx> for HirVisitor<'_, 'tcx> {
    type NestedFilter = OnlyItems;

    /// allow nested traversal
    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_impl_item(&mut self, ii: &'tcx rustc_hir::ImplItem<'tcx>) -> Self::Result {
        if let Some(const_item) = ii.opt_const_item() {
            let attrs = const_item.autozng_attrs();
            attrs.visit(&const_item, self.ctx);
        }
    }

    fn visit_item(&mut self, i: &'tcx rustc_hir::Item<'tcx>) -> Self::Result {
        if let Some(const_item) = i.opt_const_item() {
            let attrs = const_item.autozng_attrs();
            attrs.visit(&const_item, self.ctx);
        } else if let Some(alias_item) = i.opt_ty_alias() {
            let attrs = alias_item.autozng_attrs();
            attrs.visit(&alias_item, self.ctx);
        }
    }
}

pub trait RustcPublicVisitor: Sized {
    type Break;

    fn visit_crate(&mut self, krate: &Crate) -> ControlFlow<Self::Break> {
        krate.super_visit(self)
    }

    fn visit_adt(&mut self, adt: &AdtDef) -> ControlFlow<Self::Break> {
        adt.super_visit(self)
    }

    fn visit_trait(&mut self, trait_def: &TraitDef) -> ControlFlow<Self::Break> {
        trait_def.super_visit(self)
    }

    fn visit_impl(&mut self, impl_def: &rustc_public::ty::ImplDef) -> ControlFlow<Self::Break> {
        impl_def.super_visit(self)
    }

    fn visit_static(&mut self, st: &StaticDef) -> ControlFlow<Self::Break> {
        st.super_visit(self)
    }

    fn visit_fn(&mut self, fn_def: &FnDef) -> ControlFlow<Self::Break> {
        fn_def.super_visit(self)
    }

    fn visit_all_rustc_public_items(&mut self) -> ControlFlow<Self::Break> {
        // Visit all items except `CrateItems` from  `rustc_public::all_local_items()`
        // that interface is cumbersome, and the work is mostly duplicated by visiting the local `Crate`
        // visiting aliases and `const` items is better done via traversing the HIR anyway
        //
        // Only the *local* crate is walked: `Crate::super_visit` descends into its trait decls
        // (and impls), so the whole crate graph (`all_trait_decls`/`all_trait_impls`, which pull
        // in `std` and every dependency) is deliberately not visited.
        rustc_public::local_crate().visit(self)
    }
}

pub trait RustcPublicVisitable {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.super_visit(visitor)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break>;
}

impl<T: RustcPublicVisitable> RustcPublicVisitable for Vec<T> {
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for arg in self {
            arg.visit(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::Crate {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_crate(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        // visit statics (probably never used but the functionality is here)
        for st in self.statics() {
            st.visit(visitor)?;
        }
        // visit adts
        for adt in self.adts() {
            adt.visit(visitor)?;
        }
        // visit fn defs
        for fn_def in self.fn_defs() {
            fn_def.visit(visitor)?;
        }

        // visiting traits may visit come items twice is using `visit_all_rustc_public_items`
        // so be careful not to duplicate things
        for trait_def in self.trait_decls() {
            trait_def.visit(visitor)?;
        }
        for trait_impl in self.trait_impls() {
            trait_impl.visit(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::mir::mono::StaticDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_static(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::FnDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_fn(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::AdtDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_adt(self)
    }
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::TraitDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_trait(self)
    }
    /// The descent into the trait's associated functions is done by `visit_trait` (mirroring how
    /// `visit_adt` walks fields and inherent impls), so the default traversal is a no-op.
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}

impl RustcPublicVisitable for rustc_public::ty::ImplDef {
    fn visit<V: RustcPublicVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_impl(self)
    }
    /// The descent into an impl's associated items is done by `visit_impl` / `visit_adt`; the
    /// default traversal is a no-op.
    fn super_visit<V: RustcPublicVisitor>(&self, _visitor: &mut V) -> ControlFlow<V::Break> {
        ControlFlow::Continue(())
    }
}
