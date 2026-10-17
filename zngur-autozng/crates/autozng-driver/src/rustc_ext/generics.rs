use rustc_public::{
    CrateDef, DefId,
    rustc_internal::{internal, stable},
    ty::{ClauseKind, GenericArgs, Generics, Span},
};
use rustc_type_ir::UpcastFrom;

use crate::{driver::with_ctxt, rustc_ext::convert::RustcInternalEx};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GenericClauses {
    pub parent: Option<DefId>,
    pub clauses: Vec<(ClauseKind, Span)>,
}

#[derive(Debug, Clone)]
pub struct InstantiatedClauses {
    pub clauses: Vec<ClauseKind>,
    pub spans: Vec<Span>,
}

impl InstantiatedClauses {
    fn empty() -> Self {
        InstantiatedClauses {
            clauses: vec![],
            spans: vec![],
        }
    }
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }
    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }
}

impl IntoIterator for InstantiatedClauses {
    type Item = (ClauseKind, Span);

    type IntoIter = std::iter::Zip<std::vec::IntoIter<ClauseKind>, std::vec::IntoIter<Span>>;

    fn into_iter(self) -> Self::IntoIter {
        debug_assert_eq!(self.clauses.len(), self.spans.len());
        std::iter::zip(self.clauses, self.spans)
    }
}

impl<'a> IntoIterator for &'a InstantiatedClauses {
    type Item = (ClauseKind, Span);

    type IntoIter = std::iter::Zip<
        std::iter::Cloned<std::slice::Iter<'a, ClauseKind>>,
        std::iter::Copied<std::slice::Iter<'a, Span>>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        debug_assert_eq!(self.clauses.len(), self.spans.len());
        std::iter::zip(self.clauses.iter().cloned(), self.spans.iter().copied())
    }
}

impl From<rustc_public::ty::GenericClauses> for GenericClauses {
    fn from(value: rustc_public::ty::GenericClauses) -> Self {
        GenericClauses {
            parent: value.parent.map(|trait_def| trait_def.def_id()),
            clauses: value.clauses,
        }
    }
}

impl<'tcx> From<rustc_middle::ty::GenericClauses<'tcx>> for GenericClauses {
    fn from(value: rustc_middle::ty::GenericClauses<'tcx>) -> Self {
        GenericClauses {
            parent: value.parent.map(stable),
            clauses: value
                .clauses
                .iter()
                .map(|(clause, span)| (stable(clause.kind().skip_binder()), stable(span)))
                .collect(),
        }
    }
}

impl GenericClauses {
    /// instantiate self and parent clauses
    pub fn instantiate(self, args: &rustc_public::ty::GenericArgs) -> InstantiatedClauses {
        let mut instantiated = InstantiatedClauses::empty();
        self.instantiate_into(&mut instantiated, args);
        instantiated
    }

    fn instantiate_into(self, instantiated: &mut InstantiatedClauses, args: &GenericArgs) {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            if let Some(rustc_id) = self.parent {
                let parent_clauses: GenericClauses = tcx.clauses_of(rustc_id.internal(tcx)).into();
                parent_clauses.instantiate_into(instantiated, args);
            }
            let (clauses, spans): (Vec<_>, Vec<_>) = self.clauses.into_iter().unzip();
            let args = internal(tcx, args);
            instantiated
                .clauses
                .extend(clauses.into_iter().map(|clause| {
                    let clause = rustc_middle::ty::EarlyBinder::bind(
                        tcx,
                        rustc_middle::ty::Clause::upcast_from(clause.internal(tcx), tcx),
                    )
                    .instantiate(tcx, args)
                    .skip_normalization();
                    stable(clause.kind()).skip_binder()
                }));
            instantiated.spans.extend(spans);
        })
    }

    /// instantiate self and parent clauses as is
    pub fn instantiate_identity(self) -> InstantiatedClauses {
        let mut instantiated = InstantiatedClauses::empty();
        self.instantiate_identity_into(&mut instantiated);
        instantiated
    }

    fn instantiate_identity_into(self, instantiated: &mut InstantiatedClauses) {
        with_ctxt(|ctx| {
            let tcx = ctx.tcx;
            if let Some(rustc_id) = self.parent {
                let parent_clauses: GenericClauses = tcx.clauses_of(rustc_id.internal(tcx)).into();
                parent_clauses.instantiate_identity_into(instantiated);
            }
            let (clauses, spans): (Vec<_>, Vec<_>) = self.clauses.into_iter().unzip();
            instantiated.clauses.extend(clauses);
            instantiated.spans.extend(spans);
        })
    }
}

pub(crate) trait ClausesOfExt {
    fn clauses_of(&self) -> GenericClauses;
}

pub fn get_clauses_of(def_id: DefId) -> GenericClauses {
    with_ctxt(|ctx| {
        let tcx = ctx.tcx;
        let rustc_clauses = tcx.clauses_of(def_id.internal(tcx));
        GenericClauses {
            parent: rustc_clauses.parent.map(stable),
            clauses: rustc_clauses
                .clauses
                .iter()
                .map(|(clause, span)| (stable(clause.kind().skip_binder()), stable(span)))
                .collect(),
        }
    })
}

pub fn get_generics_of(def_id: DefId) -> Generics {
    with_ctxt(|ctx| stable(ctx.tcx.generics_of(def_id.internal(ctx.tcx))))
}

impl ClausesOfExt for rustc_public::ty::ImplDef {
    fn clauses_of(&self) -> GenericClauses {
        get_clauses_of(self.def_id())
    }
}

impl ClausesOfExt for rustc_public::ty::AdtDef {
    fn clauses_of(&self) -> GenericClauses {
        get_clauses_of(self.def_id())
    }
}

impl ClausesOfExt for rustc_public::ty::FnDef {
    fn clauses_of(&self) -> GenericClauses {
        get_clauses_of(self.def_id())
    }
}
