#[rustfmt::skip]
mod generated;

use generated::MyCppWrapper;

fn main() {
    let c = MyCppWrapper::new(5, 6);
    c.print();
}
