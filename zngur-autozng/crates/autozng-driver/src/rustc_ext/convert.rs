use rustc_middle::ty::TyCtxt;
use rustc_public::{CrateDef, rustc_internal::internal};

pub trait RustcInternalEx {
    type T<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx>;
}

impl RustcInternalEx for rustc_public::ty::Region {
    type T<'tcx> = rustc_middle::ty::Region<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}
impl RustcInternalEx for rustc_public::ty::Ty {
    type T<'tcx> = rustc_middle::ty::Ty<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}
impl RustcInternalEx for rustc_public::ty::TyConst {
    type T<'tcx> = rustc_middle::ty::Const<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}
impl RustcInternalEx for rustc_public::DefId {
    type T<'tcx> = rustc_span::def_id::DefId;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}
impl RustcInternalEx for Option<rustc_public::DefId> {
    type T<'tcx> = Option<rustc_span::def_id::DefId>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        self.map(|id| internal(tcx, id))
    }
}
impl RustcInternalEx for rustc_public::ty::TraitRef {
    type T<'tcx> = rustc_middle::ty::TraitRef<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}
impl RustcInternalEx for rustc_public::ty::TermKind {
    type T<'tcx> = rustc_middle::ty::Term<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        internal(tcx, self)
    }
}

impl RustcInternalEx for rustc_public::ty::ClauseKind {
    type T<'tcx> = rustc_middle::ty::ClauseKind<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        use rustc_middle::ty::ClauseKind as CK;
        use rustc_public::ty::ClauseKind;
        match self {
            ClauseKind::Trait(trait_object) => CK::Trait(trait_object.internal(tcx)),
            ClauseKind::RegionOutlives(region_outlives) => {
                CK::RegionOutlives(region_outlives.internal(tcx))
            }
            ClauseKind::TypeOutlives(type_outlives) => {
                CK::TypeOutlives(type_outlives.internal(tcx))
            }
            ClauseKind::Projection(projection_predicate) => {
                CK::Projection(projection_predicate.internal(tcx))
            }
            ClauseKind::ConstArgHasType(const_, ty) => {
                CK::ConstArgHasType(const_.internal(tcx), ty.internal(tcx))
            }
            ClauseKind::WellFormed(term) => CK::WellFormed(term.internal(tcx)),
            ClauseKind::ConstEvaluatable(const_) => CK::ConstEvaluatable(const_.internal(tcx)),
        }
    }
}

impl RustcInternalEx for rustc_public::ty::TraitClause {
    type T<'tcx> = rustc_middle::ty::TraitClause<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        rustc_middle::ty::TraitClause {
            trait_ref: self.trait_ref.internal(tcx),
            polarity: self.polarity.internal(tcx),
        }
    }
}

impl<T> RustcInternalEx for rustc_public::ty::OutlivesClause<T, rustc_public::ty::Region>
where
    T: RustcInternalEx,
{
    type T<'tcx> = rustc_middle::ty::OutlivesClause<'tcx, T::T<'tcx>>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        let rustc_public::ty::OutlivesClause(a, b) = self;
        rustc_middle::ty::OutlivesClause(a.internal(tcx), internal(tcx, b))
    }
}

impl RustcInternalEx for rustc_public::ty::ClausePolarity {
    type T<'tcx> = rustc_middle::ty::ClausePolarity;
    fn internal<'tcx>(self, _tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        use rustc_middle::ty::ClausePolarity::*;
        match self {
            rustc_public::ty::ClausePolarity::Positive => Positive,
            rustc_public::ty::ClausePolarity::Negative => Negative,
        }
    }
}

impl RustcInternalEx for rustc_public::ty::ProjectionClause {
    type T<'tcx> = rustc_middle::ty::ProjectionClause<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        let rustc_public::ty::ProjectionClause {
            projection_term,
            term,
        } = self;
        rustc_middle::ty::ProjectionClause {
            projection_term: projection_term.internal(tcx),
            term: internal(tcx, term),
        }
    }
}

impl RustcInternalEx for rustc_public::ty::AliasTerm {
    type T<'tcx> = rustc_middle::ty::AliasTerm<'tcx>;
    fn internal<'tcx>(self, tcx: TyCtxt<'tcx>) -> Self::T<'tcx> {
        // FIXME: rust-analyser, docs.rs, and rustc don't always agree on the 4th param's existence
        rustc_middle::ty::AliasTerm::new_from_def_id(
            tcx,
            self.def_id.def_id().internal(tcx),
            internal(tcx, self.args),
            // rustc_middle::ty::AliasConstInherentArgsKind::WithSelf,
        )
    }
}
