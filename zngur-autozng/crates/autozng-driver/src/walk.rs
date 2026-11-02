//! Walk filters: which interface items a recursive interface walk may visit.
//!
//! A filter is attached with `#[autozng::walk_filter(...)]` on the crate root or on an
//! emit-able item. It gates both the emission of an interface item and the walking of its
//! type. A nested filter replaces the inherited filter for its whole subtree (see
//! [`crate::attrs::AutozngAttrsExt`] and `Attr::for_child`).
//!
//! A filter is an ordered list of roots:
//!
//! ```text
//! walk_filter(exclude(TraitMethods), include(Fields, InherentMethods))
//! ```
//!
//! Each matching root sets the verdict (`all` / `include` allow, `none` / `exclude` deny) and
//! the **last matching root wins**. When no root matches, the default is the inverse of the
//! first root: `include` / `none` deny, `exclude` / `all` allow.

/// An ordered list of filter roots. The default is `all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkFilter {
    roots: Vec<WalkFilterRoot>,
}

/// One `all` / `none` / `include(...)` / `exclude(...)` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkFilterRoot {
    /// Allow every item.
    All,
    /// Deny every item.
    None,
    /// Allow items matched by at least one rule.
    Include(Vec<WalkRule>),
    /// Deny items matched by at least one rule.
    Exclude(Vec<WalkRule>),
}

/// One selector inside an `include(...)` / `exclude(...)` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkRule {
    Fields,
    Variants,
    InherentMethods,
    /// Trait methods, optionally restricted to one trait path.
    TraitMethods(Option<String>),
    Named(String),
    Receiver(ReceiverKind),
    Visibility(VisibilityKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    Value,
    Ref,
    RefMut,
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityKind {
    Public,
    PublicInternal,
    Private,
}

impl Default for WalkFilter {
    fn default() -> Self {
        Self {
            roots: vec![WalkFilterRoot::All],
        }
    }
}

impl WalkFilter {
    pub fn new(roots: Vec<WalkFilterRoot>) -> Self {
        Self { roots }
    }
}
