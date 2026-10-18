#[rustfmt::skip]
mod generated;

pub use generated::cpp::Accumulator;

fn main() {
    let mut accumulator = Accumulator::new();
    accumulator.add(20);
    accumulator.add(22);
    assert_eq!(accumulator.total(), 42);
}
