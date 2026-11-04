//@ autozng_fail
//@ error_contains: malformed `autozng::policy` attribute input
#![cfg_attr(
    autozng,
    autozng::policy(max_walk_depth = 8, max_walk_depth = 4)
)]
