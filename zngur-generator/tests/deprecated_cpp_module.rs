//! Proves that the `#[deprecated] pub type X = super::X;` compatibility
//! shim emitted inside `pub mod cpp { ... }` for old-style
//! (`type crate::X { ... }`) opaque C++ wrapper types is real: naming it
//! from consumer code actually triggers the compiler's deprecation lint.
//!
//! This is intentionally isolated from the example projects under
//! `examples/`, since this repo's CI builds those with
//! `RUSTFLAGS="-D warnings"`, which would turn the deprecation warning we
//! want to observe into a build failure of what's supposed to be a
//! working, non-deprecated example. Instead, this test invokes `rustc`
//! directly as a subprocess with its own explicit flags, independent of
//! any ambient `RUSTFLAGS`.
//!
//! `trybuild` was deliberately not used here: its default workflow does
//! exact-text `.stderr` snapshot comparison, which is brittle across this
//! repo's CI matrix (stable + nightly Rust, 3 OSes). Asserting "exit
//! status non-zero" + "stderr contains a specific substring" proves the
//! same thing without that fragility and without adding a new dependency.

use zngur_generator::*;

/// Builds a minimal spec with a single old-style (`#cpp_heap_allocated`)
/// opaque C++ wrapper type, renders it, and returns the generated Rust
/// code. `#cpp_heap_allocated` is used (rather than `#cpp_stack_owned` or
/// `#cpp_ref`) because its generated struct body has zero external crate
/// dependencies (no `zngur_lib`, no other extern symbols to resolve at
/// compile time), which makes a standalone `rustc` invocation trivial
/// with no `--extern`/link setup.
fn render_old_style_heap_allocated_type() -> String {
    let spec = ZngurSpec {
        types: vec![ZngurType {
            ty: RustType::Adt(RustPathAndGenerics {
                path: vec!["crate".to_owned(), "Way".to_owned()],
                generics: vec![],
                named_generics: vec![],
            }),
            layout: Some(LayoutPolicy::HeapAllocated),
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: Some(CppHeapAllocated("::osmium::Way".to_owned())),
            cpp_ref: None,
            cpp_stack_owned: None,
        }],
        ..Default::default()
    };
    let (rust_code, _h, _cpp) =
        ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
    rust_code
}

/// Writes a single crate-root file combining the generator's output with a
/// small consumer function that names the deprecated `cpp::Way` alias
/// purely as a type annotation (naming a deprecated item is enough to fire
/// the lint; no need to construct a value), and returns its path.
///
/// Deviation from the brief's literal recipe: the brief sketches
/// `consumer.rs` as a *separate* file that pulls in the generated code via
/// `#[path = "generated.rs"] mod generated;` and then refers to
/// `generated::cpp::Way`. In practice the generated code contains
/// crate-root-absolute paths (e.g. `crate::Way`, used internally by the
/// `#cpp_heap_allocated` bridge functions) that only resolve correctly
/// when the generated code itself sits at the crate root — exactly how
/// zngur's real generated output is used (as a crate's `lib.rs`, not
/// nested inside another module). Nesting it under `mod generated` (as the
/// brief sketches) breaks those `crate::Way` references with an unrelated
/// `E0425` error, which would falsely masquerade as/interfere with the
/// deprecation-lint assertions this test cares about. So instead, the
/// consumer function is appended directly to the end of the generated code
/// (making the combined file itself the crate root) and refers to the
/// alias as plain `cpp::Way`, matching how the deprecated shim is actually
/// meant to be named by real downstream code.
fn write_test_file(unique_suffix: &str) -> std::path::PathBuf {
    let mut combined = render_old_style_heap_allocated_type();
    combined.push_str(
        r#"

#[allow(dead_code)]
fn use_deprecated_alias(_value: cpp::Way) {}
"#,
    );

    let tmp_dir = std::env::temp_dir().join(format!(
        "zngur_deprecated_cpp_module_test_{}_{}",
        std::process::id(),
        unique_suffix
    ));
    std::fs::create_dir_all(&tmp_dir).expect("failed to create temp dir");

    let consumer_path = tmp_dir.join("consumer.rs");
    std::fs::write(&consumer_path, combined).expect("failed to write consumer.rs");

    consumer_path
}

#[test]
fn deprecated_cpp_alias_triggers_deprecation_lint_as_hard_error() {
    let consumer_path = write_test_file("deny");
    let tmp_dir = consumer_path.parent().unwrap();

    let output = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg("-D")
        .arg("deprecated")
        .arg(&consumer_path)
        .arg("-o")
        .arg(tmp_dir.join("consumer_out.rlib"))
        .output()
        .expect("failed to invoke rustc — is it on PATH?");

    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        panic!(
            "expected rustc to fail with `-D deprecated` when naming the deprecated \
             `cpp::Way` alias, but it succeeded.\n--- stderr ---\n{stderr}"
        );
    }

    assert!(
        stderr.contains("deprecated"),
        "expected rustc stderr to mention \"deprecated\", but it didn't.\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("Way"),
        "expected rustc stderr to mention the specific deprecated item (\"Way\"), \
         to confirm the failure is about this alias and not an unrelated compile error.\n\
         --- stderr ---\n{stderr}"
    );

    eprintln!("--- captured rustc stderr (with -D deprecated) ---\n{stderr}");
}

/// Positive control: proves the previous test's claim isn't vacuous by
/// compiling the exact same `consumer.rs`/`generated.rs` pair WITHOUT `-D
/// deprecated` and asserting it succeeds. Without this, a test that always
/// fails to compile for some unrelated reason (e.g. a typo in the consumer
/// file) would trivially "pass" the assertions above.
#[test]
fn deprecated_cpp_alias_compiles_fine_without_deny_deprecated() {
    let consumer_path = write_test_file("allow");
    let tmp_dir = consumer_path.parent().unwrap();

    let output = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg(&consumer_path)
        .arg("-o")
        .arg(tmp_dir.join("consumer_out.rlib"))
        .output()
        .expect("failed to invoke rustc — is it on PATH?");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "expected the generated code + consumer to compile successfully without \
         `-D deprecated` (proving the only reason the other test fails is the \
         deprecation lint, not some unrelated defect in the generated code).\n\
         --- stderr ---\n{stderr}"
    );
}
