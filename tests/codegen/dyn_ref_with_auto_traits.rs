#[rustfmt::skip]
mod generated;

pub struct Holder;

impl Holder {
    pub fn error(&self) -> &'static (dyn std::error::Error + Send + Sync) {
        Box::leak(Box::new(std::io::Error::other("boom")))
    }
}

fn main() {}
