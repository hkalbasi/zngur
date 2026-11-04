//@ autozng_fail
//@ error_contains: malformed `autozng::policy` attribute input
#![cfg_attr(autozng, autozng::policy(no_such_policy))]
