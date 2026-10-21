//! The preflight's pure decisions: parsing the driver handshake and judging it.
//!
//! These are the pass/fail policy in one place, so they are pinned without spawning a driver:
//! a toolchain mismatch is fatal, a version drift is not, and a missing required capability is
//! fatal while extra capabilities are fine.

use std::collections::BTreeSet;

use cargo_autozng::config;
use cargo_autozng::launch::preflight::{
    DriverHandshake, Verdict, candidate_channels, evaluate, parse_handshake,
};

const RUSTC: &str = "rustc 1.100.0-nightly (17fd5b8a3 2026-08-28)";
const REQUIRED: &[&str] = &["output-dir-env"];

fn handshake() -> DriverHandshake {
    DriverHandshake {
        tool_version: "0.1.0".to_string(),
        pinned_toolchain: "nightly-2026-08-29".to_string(),
        built_against_rustc: RUSTC.to_string(),
        capabilities: ["output-zng", "out-dir", "output-dir-env", "sysroot-env"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

#[test]
fn a_well_formed_handshake_round_trips() {
    let text = "\
autozng-driver 0.1.0
pinned-toolchain: nightly-2026-08-29
built-against-rustc: rustc 1.100.0-nightly (17fd5b8a3 2026-08-28)
capabilities: output-zng, out-dir, output-dir-env, sysroot-env
";
    let parsed = parse_handshake(text).expect("a valid handshake should parse");
    assert_eq!(parsed, handshake());
    assert!(parsed.capabilities.contains("output-dir-env"));
}

#[test]
fn a_foreign_or_incomplete_binary_is_not_a_handshake() {
    assert!(parse_handshake("rustc 1.100.0-nightly (17fd5b8a3 2026-08-28)").is_none());
    assert!(parse_handshake("").is_none());
    // The name matches but fields are missing.
    assert!(parse_handshake("autozng-driver 0.1.0\n").is_none());
}

#[test]
fn a_matching_driver_and_toolchain_pass() {
    assert_eq!(
        evaluate(&handshake(), RUSTC, "0.1.0", REQUIRED),
        Verdict::Ok
    );
}

#[test]
fn a_version_drift_is_a_warning_not_a_failure() {
    let Verdict::Warn(warnings) = evaluate(&handshake(), RUSTC, "0.2.0", REQUIRED) else {
        panic!("a version drift must be a warning, not a failure");
    };
    let warning = warnings.join(" ");
    assert!(warning.contains("0.1.0") && warning.contains("0.2.0"));
}

#[test]
fn a_toolchain_mismatch_is_fatal() {
    let other = "rustc 1.99.0-nightly (aaaaaaaaa 2026-06-01)";
    let Verdict::Fail(failures) = evaluate(&handshake(), other, "0.1.0", REQUIRED) else {
        panic!("a toolchain mismatch must be fatal");
    };
    let failure = failures.join(" ");
    assert!(failure.contains(RUSTC) && failure.contains(other));
}

#[test]
fn a_missing_required_capability_is_fatal() {
    let mut driver = handshake();
    driver.capabilities = BTreeSet::from(["output-zng".to_string()]);

    let Verdict::Fail(failures) = evaluate(&driver, RUSTC, "0.1.0", REQUIRED) else {
        panic!("a missing required capability must be fatal");
    };
    assert!(failures.join(" ").contains("output-dir-env"));
}

#[test]
fn candidate_channels_prefer_the_override_and_de_duplicate() {
    let known_good = config::KNOWN_GOOD_TOOLCHAIN;

    assert_eq!(candidate_channels(None), [known_good]);
    assert_eq!(
        candidate_channels(Some("nightly-2099-01-01")),
        ["nightly-2099-01-01", known_good]
    );
    // An override equal to the known-good channel is not tried twice.
    assert_eq!(candidate_channels(Some(known_good)), [known_good]);
}
