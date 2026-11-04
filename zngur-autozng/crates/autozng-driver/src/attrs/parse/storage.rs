use std::rc::Rc;

use crate::{attrs::DynAttr, rustc_ext::RustcInternalEx};

use super::*;

pub trait ToolAttrStore: Default {
    fn collect_attr(&mut self, attr: &rustc_attr_ir::Attribute) -> bool;
    fn prepare_attrs_into(&self, attrs: &mut Vec<PartialParsedAttr>);
}

#[derive(Debug, Default)]
pub struct DocAttrToolsAttrStorage {
    pub doc_lines: Vec<(String, Span)>,
}

impl ToolAttrStore for DocAttrToolsAttrStorage {
    fn collect_attr(&mut self, attr: &rustc_attr_ir::Attribute) -> bool {
        const AUTOZNG_ATTR_START: &str = const_format::formatcp!("#[{AUTOZNG_NAMESPACE}{PATH_SEP}");

        if let rustc_attr_ir::Attribute::Parsed(rustc_attr_ir::AttributeKind::DocComment {
            comment,
            span,
            style,
            kind,
        }) = attr
        {
            let comment = comment.as_str();
            let line = comment.trim();

            // doc line *looks* like a rust outer attr for autozng with only one attr per line
            if line.starts_with(AUTOZNG_ATTR_START)
                && line.ends_with("]")
                && line.matches("#[").count() == 1
            {
                let span = {
                    match kind {
                        rustc_ast::token::DocFragmentKind::Sugared(_) => {
                            // offset by prefix len + one space
                            span.offset_lo(style.line_doc_comment_prefix().len() + 1)
                        }
                        rustc_ast::token::DocFragmentKind::Raw(span) => *span,
                    }
                };

                self.doc_lines.push((comment.to_string(), span));
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn prepare_attrs_into(&self, attrs: &mut Vec<PartialParsedAttr>) {
        attrs.extend(
            self.doc_lines
                .iter()
                .filter_map(|(line, span)| parse_attr_str(line, *span).ok()),
        );
    }
}

#[derive(Debug, Default)]
pub struct RawToolAttrStorage {
    pub tools: Vec<AttrItem>,
}

impl ToolAttrStore for RawToolAttrStorage {
    fn collect_attr(&mut self, attr: &rustc_attr_ir::Attribute) -> bool {
        if let rustc_attr_ir::Attribute::Unparsed(u) = attr {
            self.tools.push(*u.clone());
            true
        } else {
            false
        }
    }

    fn prepare_attrs_into(&self, attrs: &mut Vec<PartialParsedAttr>) {
        attrs.extend(self.tools.iter().map(parse_attr_item_ir));
    }
}

macro_rules! tuple_impl_attr_store {
        ( $( $name:ident($var:ident) )+ ) => {
            impl<$($name: ToolAttrStore),+> ToolAttrStore for ($($name,)+) {
                fn collect_attr(&mut self, attr: &rustc_attr_ir::Attribute) -> bool {
                    let ($($var,)+) = self;
                    ($($var.collect_attr(attr) || )+ false)
                }
                fn prepare_attrs_into(&self, attrs: &mut Vec<PartialParsedAttr>){
                    let ($($var,)+) = self;
                    $($var.prepare_attrs_into(attrs);)+
                }
            }

        };
    }

tuple_impl_attr_store! { A(a) }
tuple_impl_attr_store! { A(a) B(b) }
tuple_impl_attr_store! { A(a) B(b) C(c) }

pub struct ToolAttrStorage<T: ToolAttrStore>(T);

impl<T: ToolAttrStore> ToolAttrStorage<T> {
    pub fn new() -> Self {
        ToolAttrStorage(T::default())
    }

    pub fn collect_attrs_for<ID: HasAttrs>(&mut self, id: ID) -> &mut Self {
        with_ctxt(|ctx| {
            // Fetch the attributes rustc actually stored for this key. `AttrKey` is deliberately
            // not a `DefId`: a `let` binding has no `DefId` of its own, so its attributes only
            // exist under its `HirId` (see `HirLetBindStmt`).
            let attrs = match id.attr_key() {
                AttrKey::DefId(def_id) => rustc_hir::attrs::HasAttrs::get_attrs(def_id, &ctx.tcx),
                AttrKey::HirId(hir_id) => rustc_hir::attrs::HasAttrs::get_attrs(hir_id, &ctx.tcx),
            };

            for attr in attrs {
                if attr
                    .path()
                    .first()
                    .is_some_and(|p| p.as_str() == AUTOZNG_NAMESPACE)
                {
                    self.0.collect_attr(attr);
                }
            }
        });
        self
    }

    pub fn prepare_attrs(&self) -> Vec<PartialParsedAttr> {
        let mut attrs = Vec::new();
        self.0.prepare_attrs_into(&mut attrs);
        attrs
    }
}

pub trait ToolStorage {
    type Store: ToolAttrStore;

    fn build() -> ToolAttrStorage<Self::Store> {
        ToolAttrStorage::new()
    }
}

pub struct WithDocCommentAttrs;

impl ToolStorage for WithDocCommentAttrs {
    type Store = (RawToolAttrStorage, DocAttrToolsAttrStorage);
}

pub struct ToolAttrsOnly;

impl ToolStorage for ToolAttrsOnly {
    type Store = RawToolAttrStorage;
}

use rustc_public::DefId;
use rustc_span::def_id::DefId as RustcDefId;

/// The identity rustc stores a list of attributes under.
///
/// Almost every item is addressed by its `DefId`, but a `let` binding inside an
/// `#[autozng::emit_items]` const has no `DefId` of its own: its `#[autozng::...]` attributes are
/// stored on its `HirId` (they are lowered with `Target::Statement`). The two are kept distinct
/// so the attribute cache can key each `let` binding separately from its enclosing item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttrKey {
    DefId(RustcDefId),
    HirId(rustc_hir::HirId),
}

impl AttrKey {
    /// The `DefId` this key addresses, if it is not a bare HIR node.
    pub fn as_def_id(self) -> Option<RustcDefId> {
        match self {
            AttrKey::DefId(def_id) => Some(def_id),
            AttrKey::HirId(_) => None,
        }
    }
}

pub trait HasAttrs {
    fn attr_key(&self) -> AttrKey;
}

impl HasAttrs for AttrKey {
    fn attr_key(&self) -> AttrKey {
        *self
    }
}

impl HasAttrs for DefId {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(with_ctxt(|ctx| self.internal(ctx.tcx)))
    }
}
impl HasAttrs for RustcDefId {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(*self)
    }
}

impl HasAttrs for rustc_hir::def_id::LocalDefId {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(self.to_def_id())
    }
}

impl HasAttrs for rustc_hir::OwnerId {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(self.to_def_id())
    }
}

impl HasAttrs for rustc_hir::HirId {
    fn attr_key(&self) -> AttrKey {
        AttrKey::HirId(*self)
    }
}

impl HasAttrs for rustc_span::def_id::CrateNum {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(self.as_def_id())
    }
}

impl HasAttrs for rustc_public::CrateNum {
    fn attr_key(&self) -> AttrKey {
        AttrKey::DefId(with_ctxt(|ctx| {
            rustc_public::rustc_internal::internal(ctx.tcx, *self).as_def_id()
        }))
    }
}

pub trait HasParentAttrs {
    fn parent_attrs(&self, attr_key: AttrKey) -> Option<Vec<Rc<dyn DynAttr>>> {
        let _ = attr_key;
        None
    }
}
