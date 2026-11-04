#[rustfmt::skip]
mod generated;

use std::any::Any;
use std::fmt::Debug;

trait DebugAny: Any + Debug {}
impl<T: Any + Debug> DebugAny for T {}

fn main() {
    let value: &(dyn DebugAny + Send) = &42i32;
    generated::debug_dyn(value);
}
