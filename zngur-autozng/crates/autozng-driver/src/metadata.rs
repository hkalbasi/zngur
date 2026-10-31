use std::hash::{DefaultHasher, Hash, Hasher};

use rustc_hash::FxHashMap;
use rustc_public::ty::GenericArgs;
use rustc_public::{CrateDef, DefId, rustc_internal::stable};
use rustc_span::Span;
use zngur_def::LayoutPolicy;

use crate::driver::with_ctxt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataKey(u64);

impl MetadataKey {
    pub fn from_public_def_id(def_id: DefId) -> Self {
        let mut hasher = DefaultHasher::default();
        def_id.hash(&mut hasher);
        MetadataKey(hasher.finish())
    }

    /// Key for one concrete monomorphization of a function item (`foo::<u8>`).
    ///
    /// A `rename` on a `let` binding inside an `#[autozng::emit_items]` const names a single
    /// monomorphization, so its export name must not leak onto the other instantiations of the
    /// same `FnDef`. The generic args (the same list recorded as `FnRecord::partial_args` /
    /// `TraitFnRecord::generics`) are folded into the key; `render` rebuilds it to look the name
    /// up. An empty arg list is the plain `FnDef` key (see [`MetadataKey::from_public_def_id`]).
    pub fn for_monomorphization(def_id: DefId, args: &GenericArgs) -> Self {
        let mut hasher = DefaultHasher::default();
        def_id.hash(&mut hasher);
        args.hash(&mut hasher);
        MetadataKey(hasher.finish())
    }
}

pub trait IntoMetadataKey {
    fn to_metadata_key(&self) -> MetadataKey;
}

impl IntoMetadataKey for MetadataKey {
    fn to_metadata_key(&self) -> MetadataKey {
        *self
    }
}

impl IntoMetadataKey for rustc_hir::OwnerId {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(stable(self.to_def_id()))
    }
}

impl IntoMetadataKey for rustc_hir::def_id::ModId {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(stable(self.to_def_id()))
    }
}

impl IntoMetadataKey for rustc_hir::def_id::LocalModId {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(stable(self.to_def_id()))
    }
}

impl IntoMetadataKey for rustc_public::ty::AssocItem {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id.def_id())
    }
}
impl IntoMetadataKey for rustc_public::ty::TraitDef {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id())
    }
}
impl IntoMetadataKey for rustc_public::ty::AdtDef {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id())
    }
}
impl IntoMetadataKey for rustc_public::ty::FnDef {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id())
    }
}
impl IntoMetadataKey for rustc_public::ty::ImplDef {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id())
    }
}
impl IntoMetadataKey for rustc_public::ty::FieldDef {
    #[inline(always)]
    fn to_metadata_key(&self) -> MetadataKey {
        MetadataKey::from_public_def_id(self.def_id())
    }
}

impl IntoMetadataKey for rustc_public::Crate {
    fn to_metadata_key(&self) -> MetadataKey {
        let def_id = with_ctxt(|ctx| {
            rustc_public::rustc_internal::stable(
                rustc_public::rustc_internal::internal(ctx.tcx, self.id)
                    .as_mod_id()
                    .to_def_id(),
            )
        });
        MetadataKey::from_public_def_id(def_id)
    }
}

/// The result of a `#[autozng::rename]`: the name to export and the span of the attribute that
/// produced it.
///
/// The span is kept so render-time diagnostics can point a `renamed here` note at the
/// offending `rename` (see `render::warn_unsupported_rename`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportName {
    pub name: String,
    /// Span of the `#[autozng::rename(...)]` attribute that produced `name`.
    pub span: Span,
}

/// A `#[autozng::layout(...)]` override recorded for a type.
///
/// Kept on the type's [`MetadataRecord`] and consulted during spec generation, where it replaces
/// the automatic `StackAllocated` layout (`render::build_type`). The span of the attribute is
/// retained so a size/alignment mismatch can point at it (see `attrs::attributes::layout`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutOverride {
    pub policy: LayoutPolicy,
    /// Span of the `#[autozng::layout(...)]` attribute that produced `policy`.
    pub span: Span,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct MetadataRecord {
    pub export_name: Option<ExportName>,
    /// `#[autozng::layout(...)]` override, if the item is a type that carries one.
    pub layout: Option<LayoutOverride>,
}

pub type MetadataMap = FxHashMap<MetadataKey, MetadataRecord>;
pub type MetadataRecordRefMut<'a> = std::cell::RefMut<'a, MetadataRecord>;
