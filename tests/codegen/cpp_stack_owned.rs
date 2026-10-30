#[rustfmt::skip]
mod generated;

pub use generated::{CfgBox, MergedCounter};

fn main() {
    let mut items = Vec::new();
    items.push(MergedCounter::new(1, 10));
    items.push(MergedCounter::new(2, 20));
    items.reserve(64);
    items[0].add(5);
    assert_eq!(items[0].get(), 15);

    let second = items.pop().unwrap();
    assert_eq!(second.into_value(), 20);

    let boxed = CfgBox::new(99);
    assert_eq!(boxed.get(), 99);
}
