#[rustfmt::skip]
#[path = "aggregation.zng.rs"]
pub mod generated;

pub use packet::Packet;

pub use generated::cpp_stats::StatsAccumulator;
