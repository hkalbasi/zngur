//! `#[autozng::policy]` — the first concrete policy, `implicitly_emit_external_interfaces`.
//!
//! By default the recursive interface pass only walks the children of *local* ADTs, so an
//! external type reached through an interface contributes just its own `type` block. The policy
//! turns on walking non-local ADTs too, so their fields/methods are declared.
//!
//! `std::net::IpAddr` is a convenient probe: it is a non-generic, public enum whose variant
//! fields are public types (`Ipv4Addr` / `Ipv6Addr`), so the policy is observable without
//! depending on std's internal layout.

use crate::harness::{PRELUDE, run, run_files};

const BODY: &str = r#"
pub struct Holder {
    pub addr: std::net::IpAddr,
}
"#;

/// A variant field of the external `IpAddr`, only present once its children are walked.
const EXTERNAL_FIELD: &str = "field 0 (offset = auto, type = ::std::net::Ipv4Addr)";
/// The field's type, only pulled in recursively once the external children are walked.
const EXTERNAL_TYPE: &str = "type ::std::net::Ipv4Addr {";

#[test]
fn without_the_policy_external_children_are_not_walked() {
    let out = run(BODY);
    let zng = out.zng();
    // `IpAddr` is exposed by `Holder`'s field, so it gets its own `type` block; its children are
    // not walked, so its variant fields (and `Ipv4Addr`) are absent.
    assert!(zng.contains("type ::std::net::IpAddr {"));
    assert!(!zng.contains(EXTERNAL_FIELD), "unexpected field in:\n{zng}");
    assert!(!zng.contains(EXTERNAL_TYPE), "unexpected type in:\n{zng}");
}

#[test]
fn the_policy_walks_external_children() {
    let source = format!(
        "{PRELUDE}#![cfg_attr(autozng, autozng::policy(implicitly_emit_external_interfaces))]\n{BODY}"
    );
    let out = run_files(&[("src/lib.rs", &source)]);
    let zng = out.zng();
    // With the policy the external ADT's children are walked, so its variant fields — and,
    // recursively, the external types they mention — are declared too.
    assert!(zng.contains(EXTERNAL_FIELD), "missing field in:\n{zng}");
    assert!(zng.contains(EXTERNAL_TYPE), "missing type in:\n{zng}");
}

#[test]
fn an_unknown_policy_is_rejected() {
    let source = format!("{PRELUDE}#![cfg_attr(autozng, autozng::policy(no_such_policy))]\n{BODY}");
    let out = run_files(&[("src/lib.rs", &source)]);
    assert!(
        out.stderr
            .contains("malformed `autozng::policy` attribute input"),
        "stderr:\n{}",
        out.stderr
    );
}
