//@ autozng

mod a {
    pub struct A;
}
pub use a::A;

mod b {
    pub struct B;
}
pub use b::*;

pub(crate) mod c {
    pub struct C;
}
pub use c::C;

mod d {
    pub struct D;
}
pub use d::D as DRenamed;

mod e {
    pub struct E;
}
mod f {
    pub use crate::e::E;
}
pub use f::E;

mod g {
    pub struct G;
}
pub mod h {
    pub use crate::g::G;
}

mod i {
    pub fn i_fn() {}
}
pub use i::i_fn;

mod j {
    pub struct J;
}

mod l {
    pub(super) struct L;
}
