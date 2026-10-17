use std::rc::Rc;

use crate::{
    attrs::{Attr, DynAttr},
    rustc_ext::RustcInternalEx,
};

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
                .filter_map(|(line, span)| parse_attr_str(&line, *span).ok()),
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
        attrs.extend(self.tools.iter().map(|attr| parse_attr_item_ir(attr)));
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
            let attrs = rustc_hir::attrs::HasAttrs::get_attrs(id.attr_key(), &ctx.tcx);

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
pub trait HasAttrs {
    fn attr_key(&self) -> RustcDefId;
}

impl HasAttrs for DefId {
    fn attr_key(&self) -> RustcDefId {
        with_ctxt(|ctx| self.internal(ctx.tcx))
    }
}
impl HasAttrs for RustcDefId {
    fn attr_key(&self) -> RustcDefId {
        *self
    }
}

impl HasAttrs for rustc_hir::def_id::LocalDefId {
    fn attr_key(&self) -> RustcDefId {
        self.to_def_id()
    }
}

impl HasAttrs for rustc_hir::OwnerId {
    fn attr_key(&self) -> RustcDefId {
        self.to_def_id()
    }
}

impl HasAttrs for rustc_hir::HirId {
    fn attr_key(&self) -> RustcDefId {
        self.owner.to_def_id()
    }
}

impl HasAttrs for rustc_span::def_id::CrateNum {
    fn attr_key(&self) -> RustcDefId {
        self.as_def_id()
    }
}

impl HasAttrs for rustc_public::CrateNum {
    fn attr_key(&self) -> RustcDefId {
        with_ctxt(|ctx| rustc_public::rustc_internal::internal(ctx.tcx, *self).as_def_id())
    }
}

pub trait HasParentAttrs {
    fn parent_attrs(&self, attr_key: RustcDefId) -> Option<Vec<Rc<dyn DynAttr>>> {
        let _ = attr_key;
        None
    }
}
