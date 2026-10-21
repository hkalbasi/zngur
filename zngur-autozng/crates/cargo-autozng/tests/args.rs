//! Argument normalisation and parsing.
//!
//! These are pure input-to-output transformations, so they are pinned here rather than checked
//! by hand: the `cargo autozng` / `cargo-autozng` equivalence, the flag table, the pass-through
//! to cargo, and the refusals.

use cargo_autozng::args::{self, Args, Command, Invocation};

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

fn generate(list: &[&str]) -> Args {
    match args::parse(&args(list)) {
        Ok(Invocation {
            command: Command::Generate,
            args,
        }) => args,
        other => panic!("expected a `generate` invocation, got {other:?}"),
    }
}

#[test]
fn strip_subcommand_handles_the_cargo_and_direct_forms() {
    let argv0 = "cargo-autozng";

    assert_eq!(
        args::strip_subcommand(args(&[argv0, "autozng", "--foo"]), "autozng"),
        ["--foo"]
    );
    assert_eq!(
        args::strip_subcommand(args(&[argv0, "--foo"]), "autozng"),
        ["--foo"]
    );
    // Only a *leading* token is stripped; a later one survives.
    assert_eq!(
        args::strip_subcommand(args(&[argv0, "--foo", "autozng"]), "autozng"),
        ["--foo", "autozng"]
    );
    assert!(args::strip_subcommand(args(&[argv0]), "autozng").is_empty());
}

#[test]
fn the_cargo_and_direct_forms_parse_identically() {
    let via_cargo = args(&[
        "cargo-autozng",
        "autozng",
        "--release",
        "-p",
        "foo",
        "--",
        "--cfg",
        "x",
    ]);
    let direct = args(&[
        "cargo-autozng",
        "--release",
        "-p",
        "foo",
        "--",
        "--cfg",
        "x",
    ]);
    assert_eq!(args::parse(&via_cargo), args::parse(&direct));
}

#[test]
fn flags_are_consumed_and_everything_else_is_forwarded() {
    let parsed = generate(&[
        "cargo-autozng",
        "generate",
        "--driver",
        "d.exe",
        "-o",
        "out",
        "--output-zng=one.zng",
        "--toolchain",
        "tc",
        "--target-dir=td",
        "--release",
        "--dry-run",
        "-v",
        "-p",
        "foo",
        "--features",
        "bar",
    ]);

    assert_eq!(parsed.driver.as_deref().unwrap().to_str(), Some("d.exe"));
    assert_eq!(parsed.output_dir.as_deref().unwrap().to_str(), Some("out"));
    assert_eq!(
        parsed.output_zng.as_deref().unwrap().to_str(),
        Some("one.zng")
    );
    assert_eq!(parsed.toolchain.as_deref().unwrap().to_str(), Some("tc"));
    assert_eq!(parsed.target_dir.as_deref().unwrap().to_str(), Some("td"));
    assert!(parsed.release && parsed.dry_run && parsed.verbose);
    assert_eq!(parsed.cargo_args, ["-p", "foo", "--features", "bar"]);
}

#[test]
fn a_double_dash_forwards_the_rest_verbatim() {
    let parsed = generate(&["cargo-autozng", "--", "--driver", "not-ours", "-o"]);
    assert!(parsed.driver.is_none());
    assert!(parsed.output_dir.is_none());
    assert_eq!(parsed.cargo_args, ["--", "--driver", "not-ours", "-o"]);
}

#[test]
fn help_version_and_no_arguments_are_commands() {
    for argv in [
        args(&["cargo-autozng"]),
        args(&["cargo-autozng", "autozng"]),
        args(&["cargo-autozng", "--help"]),
        args(&["cargo-autozng", "-h"]),
    ] {
        assert_eq!(args::parse(&argv).unwrap().command, Command::Help);
    }
    for argv in [
        args(&["cargo-autozng", "--version"]),
        args(&["cargo-autozng", "-V"]),
    ] {
        assert_eq!(args::parse(&argv).unwrap().command, Command::Version);
    }
}

#[test]
fn subcommands_that_are_not_implemented_are_refused() {
    for subcommand in ["setup", "update"] {
        let err = args::parse(&args(&["cargo-autozng", subcommand])).unwrap_err();
        assert!(
            err.contains(subcommand),
            "the error should name the subcommand: {err:?}"
        );
    }
}

#[test]
fn a_flag_without_a_value_is_an_error_naming_the_flag() {
    let err = args::parse(&args(&["cargo-autozng", "--driver"])).unwrap_err();
    assert!(err.contains("--driver"), "unexpected error: {err:?}");
}
