//@ autozng
//@ no_stderr
#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]
pub struct Holder {
    pub addr: std::net::IpAddr,
}
