//@ autozng
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces, max_external_depth = 0))]

// A re-exported type that was walked at the full budget must keep its fields and methods when a
// later request reaches it at the end of the budget: the truncated request must not replace the
// recorded interface.

pub use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
