//@ autozng

// A `walk_filter` on an `emit_items` const applies to the enumerated item's subtree and replaces
// the built-in slice / `str` method defaults.
//
// `bytes` keeps the method that exposes the slice; `chunks` replaces the six default slice
// methods with just the one it names.

pub struct Widget;

impl Widget {
    pub fn bytes(&self) -> &[u8] {
        &[]
    }
}

#[cfg(autozng)]
#[autozng::emit_items]
#[autozng::walk_filter(include(Named("bytes"), Named("chunks")))]
const _: [Widget; 0] = [];
