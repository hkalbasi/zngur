//! The *visible* path of a local definition.
//!
//! The `.zng` names a type by the path the generated code uses, which is the path it is
//! *reachable* by, not where it happens to be defined. A crate that keeps its modules private and
//! re-exports its API (`mod common; pub use common::*;`) must be spelled the way the re-export
//! exposes it (`crate::ColorType`), not by its definition path (`crate::common::ColorType`).
//!
//! Only public `pub use` re-exports are considered; when several expose the same definition, the
//! shortest path wins.

use rustc_hash::FxHashMap;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::DefId;
use rustc_hir::intravisit::{self, Visitor};
use rustc_hir::{Item, ItemKind, UseKind};
use rustc_middle::ty::TyCtxt;
use rustc_public::CrateDef;
use rustc_span::def_id::LocalModId;

use crate::driver::with_ctxt;

use super::vis::enclosing_modules_are_all_public;

/// A map from a definition to the path a public re-export exposes it by, as `.zng` segments (the
/// first segment is the crate name).
pub(crate) type VisiblePaths = FxHashMap<DefId, Vec<String>>;

/// Build [`VisiblePaths`] by walking the crate's `use` items.
pub(crate) fn compute(tcx: TyCtxt<'_>) -> VisiblePaths {
    let mut collector = Collector {
        tcx,
        uses: Vec::new(),
        module_items: FxHashMap::default(),
    };
    tcx.hir_visit_all_item_likes_in_crate(&mut collector);

    let mut map = VisiblePaths::default();
    for (module, item) in &collector.uses {
        // Only a *public* re-export exposes the item under the re-exported name, and the path is
        // part of the public interface only when every module on the way is `pub`.
        if !tcx.visibility(item.owner_id.def_id).is_public()
            || !enclosing_modules_are_all_public(tcx, item.owner_id.def_id)
        {
            continue;
        }
        let ItemKind::Use(use_path, use_kind) = &item.kind else {
            continue;
        };
        let module_path = module_path(tcx, *module);
        // A single import can name a type or a value (a function), so both namespaces matter.
        let Some(res) = use_path.res.type_ns.or(use_path.res.value_ns) else {
            continue;
        };

        match use_kind {
            UseKind::Single(ident) => {
                let Some(def_id) = res.opt_def_id() else {
                    continue;
                };
                let mut path = module_path;
                path.push(ident.to_string());
                insert(&mut map, def_id, path);
            }
            UseKind::Glob => {
                let Res::Def(DefKind::Mod, module_def_id) = res else {
                    continue;
                };
                let Some(local) = module_def_id.as_local() else {
                    continue;
                };
                let Some(children) = collector
                    .module_items
                    .get(&LocalModId::new_unchecked(local))
                else {
                    continue;
                };
                for def_id in children {
                    let name = tcx
                        .def_path_str(*def_id)
                        .rsplit("::")
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let mut path = module_path.clone();
                    path.push(name);
                    insert(&mut map, *def_id, path);
                }
            }
            // A degenerate list import (`use foo::{a, b}`); the leaves are separate `Single`
            // imports, so there is nothing to record here.
            UseKind::ListStem => {}
        }
    }

    map
}

/// The `.zng` path segments for `def`: the visible (re-export) path when one exists, else the
/// definition path. The first segment is the crate name; callers replace it with `crate` for a
/// local definition.
pub(crate) fn visible_path(def: &impl CrateDef) -> Vec<String> {
    let mut path: Vec<String> = def.name().split("::").map(Into::into).collect();
    if def.krate().is_local
        && let Some(visible) = with_ctxt(|ctx| {
            ctx.visible_path(rustc_public::rustc_internal::internal(
                ctx.tcx,
                def.def_id(),
            ))
        })
    {
        path = visible;
    }
    path
}

/// Collects the crate's `use` items and, for glob expansion, the items each module declares.
struct Collector<'tcx> {
    tcx: TyCtxt<'tcx>,
    /// `use` items, paired with the module they live in.
    uses: Vec<(LocalModId, &'tcx Item<'tcx>)>,
    /// Items declared in each module: module -> [definition].
    module_items: FxHashMap<LocalModId, Vec<DefId>>,
}

impl<'tcx> Visitor<'tcx> for Collector<'tcx> {
    fn visit_item(&mut self, item: &'tcx Item<'tcx>) {
        let module = self.tcx.parent_module_from_def_id(item.owner_id.def_id);
        if matches!(item.kind, ItemKind::Use(..)) {
            self.uses.push((module, item));
        } else {
            self.module_items
                .entry(module)
                .or_default()
                .push(item.owner_id.to_def_id());
        }
        intravisit::walk_item(self, item);
    }
}

/// The `.zng` path segments of a module (the first is the crate name).
///
/// `def_path_str` omits the crate name for local modules (the root is `""`, `h` is `"h"`), so it
/// is prepended to match what `CrateDef::name` returns for a definition.
fn module_path(tcx: TyCtxt<'_>, module: LocalModId) -> Vec<String> {
    let mut path = vec![tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).to_string()];
    path.extend(
        tcx.def_path_str(module.to_def_id())
            .split("::")
            .filter(|segment| !segment.is_empty())
            .map(str::to_string),
    );
    path
}

/// Record `path` for `def_id`, keeping the shortest path when several re-exports expose it.
fn insert(map: &mut VisiblePaths, def_id: DefId, path: Vec<String>) {
    match map.get(&def_id) {
        Some(existing) if existing.len() <= path.len() => {}
        _ => {
            map.insert(def_id, path);
        }
    }
}
