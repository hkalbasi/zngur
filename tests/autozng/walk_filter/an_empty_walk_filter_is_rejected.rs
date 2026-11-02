//@ autozng_fail
//@ error_contains: malformed `autozng::walk_filter` attribute input
#![cfg_attr(autozng, autozng::walk_filter())]
