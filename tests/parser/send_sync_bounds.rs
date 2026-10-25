#[rustfmt::skip]
mod generated;

use std::sync::Arc;

fn call_boxed(callback: Box<dyn Fn(i32) -> i32 + Send + Sync>, value: i32) -> i32 {
    callback(value)
}

fn make_arc() -> Arc<dyn Fn(i32) -> i32 + Send + Sync> {
    Arc::new(|value| value + 3)
}

fn call_arc(callback: Arc<dyn Fn(i32) -> i32 + Send + Sync>, value: i32) -> i32 {
    callback(value)
}

fn main() {
    assert_eq!(generated::verify_send_sync_bounds(), 1);
}
