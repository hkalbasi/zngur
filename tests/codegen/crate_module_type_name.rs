#[rustfmt::skip]
mod generated;

#[derive(Debug, PartialEq, Eq)]
enum Unit {
    Unspecified,
    Meter,
}

fn main() {
    assert_eq!(
        generated::use_units(()),
        Unit::Unspecified
    );
}
