//@ autozng

// A slice / `str` type block exposes the essential inherent methods, so C++ can build a span and
// use it without index support. The impl is walked in full; a default `walk_filter` keeps only
// the essentials (the rest of the API fans out into iterators and other std types).

pub struct Buffers;

impl Buffers {
    pub fn bytes(&self) -> &[u8] {
        &[1, 2, 3]
    }

    pub fn words(&self) -> &[u32] {
        &[1, 2]
    }

    pub fn text(&self) -> &str {
        "hi"
    }
}
