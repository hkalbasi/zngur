//@ autozng_fail
//@ error_contains: malformed `autozng::policy` attribute input
#![cfg_attr(
    autozng,
    autozng::policy(implicitly_emit_external_interfaces = 1)
)]
