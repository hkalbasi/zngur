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
                match module_def_id.as_local() {
                    // A glob of a local module: the collector already knows the module's items.
                    Some(local) => {
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
                    // A glob of *another* crate's module (`pub use png::*;`): the items are
                    // defined elsewhere, so this crate's HIR has no record of them.
                    None => collect_external_glob(tcx, module_def_id, &module_path, &mut map),
                }
            }
            // A degenerate list import (`use foo::{a, b}`); the leaves are separate `Single`
            // imports, so there is nothing to record here.
            UseKind::ListStem => {}
        }
    }

    map
}

/// The `.zng` path segments for `def`.
///
/// A definition the analysed crate can name from its own root — a local one, or an external one a
/// public `pub use` re-exports — is spelled `crate::...`: Zngur's generated code is compiled
/// inside the crate (`mod generated;`), so that is the path it uses. A definition the crate
/// cannot name keeps its own crate's path, which the printer renders with a leading `::`.
pub(crate) fn visible_path(def: &impl CrateDef) -> Vec<String> {
    let visible = with_ctxt(|ctx| {
        ctx.visible_path(rustc_public::rustc_internal::internal(
            ctx.tcx,
            def.def_id(),
        ))
    });
    // A re-exported definition is nameable from the crate root even when it is defined in another
    // crate, so its re-export path is localized exactly like a local definition's.
    let nameable_from_root = visible.is_some() || def.krate().is_local;
    let mut path = visible.unwrap_or_else(|| def.name().split("::").map(Into::into).collect());
    if nameable_from_root && !path.is_empty() {
        path[0] = "crate".to_string();
    }
    path
}

/// The definitions in `paths` that live in *another* crate, in a deterministic order.
///
/// These are the items a `pub use` makes part of this crate's public API without defining them
/// here. The crate walk only visits the local crate, so they are recorded by a dedicated pass
/// (`analyze::visit_reexported_items`) rather than by the visitor.
pub(crate) fn external_reexports(paths: &VisiblePaths) -> Vec<DefId> {
    let mut defs: Vec<DefId> = paths
        .keys()
        .copied()
        .filter(|def_id| def_id.as_local().is_none())
        .collect();
    defs.sort_by_cached_key(|def_id| paths.get(def_id).cloned());
    defs
}

/// Record the public items a glob import of an *external* module (`pub use png::*;`) brings into
/// this crate, under the path the re-export exposes them by.
///
/// The items are defined in another crate, so this crate's HIR has no record of them; the
/// module's *resolved* children are read from its metadata instead. A public submodule is
/// descended into, because re-exporting a module also makes its public contents nameable through
/// the re-exporting crate (`crate::text_metadata::TEXtChunk`).
fn collect_external_glob(
    tcx: TyCtxt<'_>,
    module_def_id: DefId,
    module_path: &[String],
    map: &mut VisiblePaths,
) {
    for child in tcx.module_children(module_def_id) {
        // Only the module's public interface is re-exported. A private child (`png::common` is
        // `Restricted`) is reachable only through the re-export of the items it exposes.
        if !child.vis.is_public() {
            continue;
        }
        let Some(def_id) = child.res.opt_def_id() else {
            continue;
        };
        let seen = map.contains_key(&def_id);
        let mut path = module_path.to_vec();
        path.push(child.ident.to_string());
        insert(map, def_id, path.clone());
        // A module reached through several re-exports needs descending into only once.
        if !seen && matches!(child.res, Res::Def(DefKind::Mod, _)) && def_id.as_local().is_none() {
            collect_external_glob(tcx, def_id, &path, map);
        }
    }
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
