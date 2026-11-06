use std::panic::catch_unwind;

use expect_test::{Expect, expect};
use zngur_def::{LayoutPolicy, RustPathAndGenerics, RustType, ZngurSpec};

use crate::{
    DefaultImportResolver, EntityPath, ImportResolver, ParsedZngFile, ReportSink,
    cfg::{InMemoryRustCfgProvider, NullCfg, RustCfgProvider},
};

/// Parse a .zng file from a string.
fn parse_str(
    text: &str,
    cfg: impl RustCfgProvider + 'static,
    warning_sink: impl FnMut(&str),
) -> crate::ParseResult {
    let mut error_sink = TestErrorSink::new(warning_sink);
    ParsedZngFile::parse_str_with_resolver(
        text,
        "test.zng",
        cfg,
        &DefaultImportResolver,
        &mut error_sink,
    )
}

fn parse_str_with_resolver(
    text: &str,
    cfg: impl RustCfgProvider + 'static,
    resolver: &impl ImportResolver,
    warning_sink: impl FnMut(&str),
) -> crate::ParseResult {
    let mut report_sink = TestErrorSink::new(warning_sink);
    ParsedZngFile::parse_str_with_resolver(text, "test.zng", cfg, resolver, &mut report_sink)
}

#[derive(Debug)]
struct TestErrorSink<F>
where
    F: FnMut(&str),
{
    error_buffer: Vec<u8>,
    warning_sink: F,
}

impl<F> TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn new(f: F) -> Self {
        Self {
            error_buffer: Default::default(),
            warning_sink: f,
        }
    }
}

impl<F> ReportSink for TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn sink_report(&mut self, report: &crate::ReportEntry, source_cache: crate::SourceCache) {
        if report.fatal {
            report
                .report
                .write(source_cache, &mut self.error_buffer)
                .unwrap();
        } else {
            let mut buf = Vec::<u8>::new();
            report.report.write(source_cache, &mut buf).unwrap();
            (self.warning_sink)(&String::from_utf8(strip_ansi_escapes::strip(buf)).unwrap());
        };
    }
}

/// unwind with the full error on drop if not empty
impl<F> Drop for TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn drop(&mut self) {
        if !self.error_buffer.is_empty() {
            std::panic::resume_unwind(Box::new(ErrorText(
                String::from_utf8(strip_ansi_escapes::strip(&self.error_buffer)).unwrap(),
            )))
        }
    }
}

fn check_success(zng: &str) {
    let _ = parse_str(zng, NullCfg, |msg| {
        unreachable!("an unexpected warning was emitted: {msg}");
    });
}

#[track_caller]
fn check_fail(zng: &str, expected: Expect) {
    let r = catch_unwind(|| {
        let _ = parse_str(zng, NullCfg, |_| {});
    });
    match r {
        Ok(_) => panic!("Parsing unexpectedly succeeded"),
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => expected.assert_eq(&t.0),
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

#[track_caller]
fn check_import_fail(zng: &str, expected: Expect, resolver: &MockFilesystem) {
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(zng, NullCfg, resolver, |_| {});
    });
    match r {
        Ok(_) => panic!("Parsing unexpectedly succeeded"),
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => expected.assert_eq(&t.0),
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

pub struct ErrorText(pub String);

// useful for debugging a test that should succeeded on parse
fn catch_parse_fail(
    zng: &str,
    cfg: impl RustCfgProvider + std::panic::UnwindSafe + 'static,
) -> crate::ParseResult {
    let r = catch_unwind(move || parse_str(zng, cfg, |_| {}));

    match r {
        Ok(r) => r,
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => {
                eprintln!("{}", t.0);
                crate::ParseResult {
                    spec: ZngurSpec::default(),
                    processed_files: Vec::new(),
                    errors: 1,
                    warnings: 0,
                }
            }
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

#[test]
fn parse_tuple() {
    check_success(
        r#"
type (i8, u8) {
    #layout(size = 0, align = 1);
}
    "#,
    );
}

macro_rules! assert_ty_path {
    ($path_expected:expr, $ty:expr) => {{
        let RustType::Adt(RustPathAndGenerics { path: p, .. }) = $ty else {
            panic!("type `{:?}` is not a path", $ty);
        };
        assert_eq!(p.as_slice(), $path_expected);
    }};
}

#[test]
fn alias_expands_correctly() {
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;
type MyString {
    #layout(size = 24, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let RustType::Adt(RustPathAndGenerics { path: p, .. }) = &ty.ty else {
        panic!("no match?");
    };
    assert_eq!(p.as_slice(), ["std", "string", "String"]);
}

#[test]
fn alias_expands_nearest_scope_first() {
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;
mod crate {
    use MyLocalString as MyString;
    type MyString {
        #layout(size = 24, align = 8);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let RustType::Adt(RustPathAndGenerics { path: p, .. }) = &ty.ty else {
        panic!("no match?");
    };
    assert_eq!(p.as_slice(), ["crate", "MyLocalString"]);
}

#[test]
fn parse_variants() {
    println!("meow");

    let parsed = parse_str(
        r#"
type Option<i32> {
    #layout(size = 8, align = 4);
    variant None { }
    variant Some {
        field 0 (offset = auto, type = i32);
    }
}
        "#,
        NullCfg,
        |_| {},
    );

    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_eq!(ty.variants.len(), 2, "should have two variants");

    assert_eq!(ty.variants[0].name, "None");
    assert_eq!(ty.variants[0].fields.len(), 0);

    assert_eq!(ty.variants[1].name, "Some");
    assert_eq!(ty.variants[1].fields.len(), 1);
    assert_eq!(ty.variants[1].fields[0].name, "0");
    assert_eq!(
        ty.variants[1].fields[0].ty,
        RustType::Primitive(zngur_def::PrimitiveRustType::Int(32))
    );
    assert_eq!(ty.variants[1].fields[0].offset, None);
}

struct MockFilesystem {
    files: std::collections::HashMap<std::path::PathBuf, String>,
}

impl MockFilesystem {
    fn new(
        files: impl IntoIterator<Item = (impl Into<std::path::PathBuf>, impl Into<String>)>,
    ) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

impl ImportResolver for MockFilesystem {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd.join(relpath);
        self.files
            .get(&path)
            .cloned()
            .ok_or_else(|| format!("File not found: {}", path.display()))
    }
}

#[test]
fn import_parser_test() {
    let resolver = MockFilesystem::new(vec![(
        "./relative/path.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./relative/path.zng";
type Example {
    #layout(size = 1, align = 1);
}
    "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(parsed.spec.types.len(), 2);
}

#[test]
fn convert_panic_to_exception_in_main_file_succeeds() {
    check_success(
        r#"
#convert_panic_to_exception
type A {
    #layout(size = 1, align = 1);
}
        "#,
    );
}

// Tests for processed_files tracking (depfile support)

#[test]
fn processed_files_single_file() {
    let parsed = parse_str(
        r#"
type A {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        |_| {},
    );
    // Should have exactly one file (test.zng)
    assert_eq!(parsed.processed_files.len(), 1);
    assert_eq!(
        parsed.processed_files[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap(),
        "test.zng"
    );
}

#[test]
fn processed_files_with_import() {
    let resolver = MockFilesystem::new(vec![(
        "./imported.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./imported.zng";
type Main {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    // Should have two files: main (test.zng) + imported
    assert_eq!(parsed.processed_files.len(), 2);
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"imported.zng"));
}

#[test]
fn processed_files_with_nested_imports() {
    let resolver = MockFilesystem::new(vec![
        (
            "./a.zng",
            r#"merge "./b.zng"; type A { #layout(size = 1, align = 1); }"#,
        ),
        (
            "./b.zng",
            r#"merge "./c.zng"; type B { #layout(size = 1, align = 1); }"#,
        ),
        ("./c.zng", "type C { #layout(size = 1, align = 1); }"),
    ]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type Main {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    // Should have four files: main + a + b + c
    assert_eq!(parsed.processed_files.len(), 4);
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"a.zng"));
    assert!(file_names.contains(&"b.zng"));
    assert!(file_names.contains(&"c.zng"));
}

fn assert_layout(wanted_size: usize, wanted_align: usize, layout: &Option<LayoutPolicy>) {
    if !matches!(layout, Some(LayoutPolicy::StackAllocated { size, align }) if *size == wanted_size && *align == wanted_align)
    {
        panic!(
            "no match: StackAllocated {{ size: {wanted_size}, align: {wanted_align} }} != {:?} ",
            layout
        );
    };
}

static EMPTY_CFG: [(&str, &[&str]); 0] = [];

#[test]
fn test_if_conditional_type_item() {
    let source = r#"
#unstable(cfg_if)

type ::std::string::String {
    #if cfg!(target_pointer_width = "64") {
        #layout(size = 24, align = 8);
    } #else if cfg!(target_pointer_width = "32") {
        #layout(size = 12, align = 4);
    } #else {
        // silly size for testing
        #layout(size = 48, align = 16);
    }
}
    "#;
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["64"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(24, 8, &ty.layout);
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["32"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(12, 4, &ty.layout);
    let parsed = catch_parse_fail(source, NullCfg);
    let ty = parsed.spec.types.first().expect("no type parsed");

    assert_layout(48, 16, &ty.layout);
}

#[test]
fn test_match_conditional_type_item() {
    let source = r#"
#unstable(cfg_match)

type ::std::string::String {
    #match cfg!(target_pointer_width) {
        // single item arm
        "64" => #layout(size = 24, align = 8);
        // match usize numbers
        32 => {
            #layout(size = 12, align = 4);
        },

        _ => {
            // silly size for testing
            #layout(size = 48, align = 16);
        }
    }
}
    "#;
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["64"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(24, 8, &ty.layout);
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["32"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(12, 4, &ty.layout);
    let parsed = catch_parse_fail(source, NullCfg);
    let ty = parsed.spec.types.first().expect("no type parsed");

    assert_layout(48, 16, &ty.layout);
}

macro_rules! test_paths_with_cfg {
    ($src:expr, $features_path_pairs:expr) => {
        for (cfg, path) in $features_path_pairs.iter().map(|(cfg, path)| {
            (
                cfg.into_iter().copied().collect::<Vec<_>>(),
                path.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
        }) {
            let parsed =
                catch_parse_fail($src, InMemoryRustCfgProvider::default().with_values(cfg));
            let ty = parsed.spec.types.first().expect("no type parsed");
            assert_ty_path!(path, &ty.ty);
        }
    };
}

type CfgPathPairs<'a> = &'a [(&'a [(&'a str, &'a [&'a str])], &'a [&'a str])];

#[test]
fn conditional_if_spec_item() {
    let source = r#"
#unstable(cfg_if)

#if cfg!(feature = "foo") {
    type crate::Foo {
        #layout(size = 1, align = 1);
    }
} #else {
    type crate::Bar {
        #layout(size = 1, align = 1);
    }
}
    "#;

    let pairs: CfgPathPairs = &[
        (&[("feature", &["foo"])], &["crate", "Foo"]),
        (&EMPTY_CFG, &["crate", "Bar"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn conditional_match_spec_item() {
    let source = r#"
#unstable(cfg_match)

#match cfg!(feature) {
    "foo" => type crate::Foo {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::Bar {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (&[("feature", &["foo"])], &["crate", "Foo"]),
        (&EMPTY_CFG, &["crate", "Bar"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn match_pattern_single_cfg() {
    let source = r#"
#unstable(cfg_match)

#match cfg!(feature) {
    "bar" | "zigza" => type crate::BarZigZa {
        #layout(size = 1, align = 1);
    }
    // match two values from a cfg value as a set
    "foo" & "baz" => type crate::FooBaz {
        #layout(size = 1, align = 1);
    }
    // negative matching (no feature baz)
    "foo" & !"baz" => type crate::FooNoBaz {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::Zoop {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (&EMPTY_CFG, &["crate", "Zoop"]),
        (&[("feature", &["foo"])], &["crate", "FooNoBaz"]),
        (&[("feature", &["bar"])], &["crate", "BarZigZa"]),
        (&[("feature", &["zigza"])], &["crate", "BarZigZa"]),
        (&[("feature", &["foo", "baz"])], &["crate", "FooBaz"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn if_pattern_multi_cfg() {
    let source = r#"
#unstable(cfg_if)
// match two cfg keys as a set
#if cfg!(feature.foo) && cfg!(target_pointer_width = 32) {
    type crate::Foo32 {
        #layout(size = 1, align = 1);
    }
} #else if cfg!(feature.foo = None) && cfg!(target_pointer_width = 64) {
    type crate::NoFoo64 {
        #layout(size = 1, align = 1);
    }
} #else if (cfg!(feature.foo = Some) && cfg!(target_pointer_width = 64)) || cfg!(feature.baz) {
    type crate::Foo64_OrBaz {
        #layout(size = 1, align = 1);
    }
} #else {
    type crate::SpecialFoo {
        #layout(size = 1, align = 1);
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (
            &[("target_pointer_width", &["32"]), ("feature", &["foo"])],
            &["crate", "Foo32"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["bar"])],
            &["crate", "NoFoo64"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["foo"])],
            &["crate", "Foo64_OrBaz"],
        ),
        (
            &[("target_pointer_width", &["32"]), ("feature", &["baz"])],
            &["crate", "Foo64_OrBaz"],
        ),
        (&[("feature", &["foo"])], &["crate", "SpecialFoo"]),
        (&EMPTY_CFG, &["crate", "SpecialFoo"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn match_pattern_multi_cfg() {
    let source = r#"
#unstable(cfg_match)

#match (cfg!(feature.foo), cfg!(target_pointer_width)) {
    // match two cfg keys as a set
    (Some, "32") => type crate::Foo32 {
        #layout(size = 1, align = 1);
    }
    (None, 64) => type crate::NoFoo64 {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::SpecialFoo {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (
            &[("target_pointer_width", &["32"]), ("feature", &["foo"])],
            &["crate", "Foo32"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["bar"])],
            &["crate", "NoFoo64"],
        ),
        (&[("feature", &["foo"])], &["crate", "SpecialFoo"]),
        (&EMPTY_CFG, &["crate", "SpecialFoo"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn module_import_parser_test() {
    let parsed = parse_str(
        r#"
import "module.zng";
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.imported_modules.len(), 1);
    assert_eq!(
        parsed.spec.imported_modules[0].path.to_str().unwrap(),
        "module.zng"
    );
}

#[test]
fn cpp_additional_includes() {
    let parsed = parse_str(
        r#"
#cpp_additional_includes "
    // comment
    stuff
"
    "#,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.additional_includes.0,
        "\n    // comment\n    stuff\n"
    );

    let parsed = parse_str(
        r##"
#cpp_additional_includes r#"
    // comment
    "stuff"
"#
    "##,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.additional_includes.0,
        "\n    // comment\n    \"stuff\"\n"
    );
}

// Tests for `c++::a::b::Name` paths.

fn assert_cpp(expected: &[&str], ty: &RustType) {
    let RustType::Cpp(segs) = ty else {
        panic!("type `{:?}` is not a c++ type", ty);
    };
    assert_eq!(segs.as_slice(), expected);
}

#[test]
fn mod_cpp_shorthand_matches_explicit_cpp_path() {
    let direct = parse_str(
        r#"
type c++::a::b::Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let via_mod = parse_str(
        r#"
mod c++::a::b {
    type Name {
        #layout(size = 16, align = 8);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let direct_ty = &direct.spec.types.first().expect("no type parsed").ty;
    let via_mod_ty = &via_mod.spec.types.first().expect("no type parsed").ty;
    assert_cpp(&["a", "b", "Name"], direct_ty);
    assert_eq!(direct_ty, via_mod_ty);
}

#[test]
fn alias_can_target_a_cpp_path() {
    // Unlike a method's `use` path or a trait bound, a `use ... as` alias
    // target can legitimately be a `c++::` path -- referencing the alias
    // elsewhere should resolve to exactly that `c++::` path.
    let parsed = parse_str(
        r#"
use c++::a::Foo as MyFoo;

type MyFoo {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "Foo"], &ty.ty);
}

#[test]
fn cpp_path_rejected_in_method_use_path() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 1, align = 1);
    fn bar(self) -> usize use c++::a::Bar;
    fn baz(self) -> usize use c++::a::Bar as cpp_baz;
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:4:31 ]
               │
             4 │     fn bar(self) -> usize use c++::a::Bar;
               │                               ─────┬─────  
               │                                    ╰─────── `c++::` paths cannot be used in a method's `use` path
            ───╯
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:5:31 ]
               │
             5 │     fn baz(self) -> usize use c++::a::Bar as cpp_baz;
               │                               ─────┬─────  
               │                                    ╰─────── `c++::` paths cannot be used in a method's `use` path
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_rejected_as_trait() {
    check_fail(
        r#"
trait c++::Foo {
}

extern "C++" {
    impl c++::Foo for crate::X {
    }
    safe fn take(&dyn c++::Foo);
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:2:7 ]
               │
             2 │ trait c++::Foo {
               │       ────┬───  
               │           ╰───── `c++::` paths cannot be used as a trait
            ───╯
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:6:10 ]
               │
             6 │     impl c++::Foo for crate::X {
               │          ────┬───  
               │              ╰───── `c++::` paths cannot be used as a trait
            ───╯
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:8:23 ]
               │
             8 │     safe fn take(&dyn c++::Foo);
               │                       ────┬───  
               │                           ╰───── `c++::` paths cannot be used as a trait
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_via_alias_indirection_rejected_as_trait() {
    check_fail(
        r#"
use c++::Foo as MyTrait;

extern "C++" {
    impl MyTrait for crate::X {
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:5:10 ]
               │
             5 │     impl MyTrait for crate::X {
               │          ───┬───  
               │             ╰───── `c++::` paths cannot be used as a trait
            ───╯
        "#]],
    );
}

#[test]
fn relative_trait_inside_cpp_mod_rejected_with_help() {
    check_fail(
        r#"
mod c++::ns {
    extern "C++" {
        impl std::fmt::Debug for Inventory {
        }
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:4:14 ]
               │
             4 │         impl std::fmt::Debug for Inventory {
               │              ───────┬───────  
               │                     ╰───────── `c++::` paths cannot be used as a trait
               │ 
               │ Help: inside a `c++::` module, prefix Rust traits with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_alias_and_relative_use_rejected_in_method_use_path() {
    check_fail(
        r#"
use c++::a::Bar as MyBar;

type crate::Foo {
    #layout(size = 1, align = 1);
    fn bar(self) -> usize use MyBar;
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:6:31 ]
               │
             6 │     fn bar(self) -> usize use MyBar;
               │                               ──┬──  
               │                                 ╰──── `c++::` paths cannot be used in a method's `use` path
            ───╯
        "#]],
    );
    check_fail(
        r#"
mod c++::a {
    type Foo {
        fn bar(&self) use std::ops::Deref;
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:4:27 ]
               │
             4 │         fn bar(&self) use std::ops::Deref;
               │                           ───────┬───────  
               │                                  ╰───────── `c++::` paths cannot be used in a method's `use` path
               │ 
               │ Help: inside a `c++::` module, prefix Rust paths with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_with_generic_arguments_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn make() -> c++::Vec<i32>;
}
    "#,
        expect![[r#"
            Error: `c++::` paths do not support generic arguments
               ╭─[ test.zng:3:23 ]
               │
             3 │     safe fn make() -> c++::Vec<i32>;
               │                       ────┬───  
               │                           ╰───── `c++::` paths do not support generic arguments
            ───╯
        "#]],
    );
    check_fail(
        r#"
mod c++::ns {
    extern "C++" {
        safe fn make() -> Option<i32>;
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths do not support generic arguments
               ╭─[ test.zng:4:27 ]
               │
             4 │         safe fn make() -> Option<i32>;
               │                           ───┬──  
               │                              ╰──── `c++::` paths do not support generic arguments
               │ 
               │ Help: inside a `c++::` module, prefix Rust paths with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_allowed_as_impl_target() {
    check_success(
        r#"
extern "C++" {
    impl c++::Foo {
    }
}
    "#,
    );
}

#[test]
fn cpp_path_allowed_as_impl_for_trait_target() {
    check_success(
        r#"
extern "C++" {
    impl crate::SomeTrait for c++::Foo {
    }
}
    "#,
    );
}

#[test]
fn cpp_path_lexer_tolerates_whitespace_between_tokens() {
    // `c++` is lexed as a single, indivisible token (like `->` or `::`), so
    // whitespace *inside* it (`c ++`) does not lex as `Token::CppPathStart` --
    // it falls back to `Ident("c")` + `Plus` + `Plus`, same as any other
    // unrecognized punctuation sequence would. But, like every other token in
    // this grammar (e.g. `crate ::`), ordinary whitespace *between* the
    // `c++` token and the following `::`/segments is fine.
    let normal = parse_str(
        r#"
type c++::Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let spaced = parse_str(
        r#"
type c++ :: Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let normal_ty = &normal.spec.types.first().expect("no type parsed").ty;
    let spaced_ty = &spaced.spec.types.first().expect("no type parsed").ty;
    assert_cpp(&["Name"], normal_ty);
    assert_eq!(normal_ty, spaced_ty);
}

#[test]
fn cpp_path_with_space_inside_token_is_a_clean_syntax_error_not_a_panic() {
    // `c ++ :: Name` (space between `c` and `++`) does NOT lex as the `c++`
    // path-start token -- it lexes as `Ident("c")` followed by `Plus`, `Plus`,
    // which is a plain syntax error (not a panic).
    check_fail(
        r#"
type c ++ :: Name {
    #layout(size = 16, align = 8);
}
    "#,
        expect![[r#"
            Error: found '+' expected '::', '<', or '{'
               ╭─[ test.zng:2:8 ]
               │
             2 │ type c ++ :: Name {
               │        ┬  
               │        ╰── found '+' expected '::', '<', or '{'
            ───╯
        "#]],
    );
}

#[test]
fn ordinary_path_starting_with_c_is_unaffected() {
    let parsed = parse_str(
        r#"
type crate::config::Foo {
    #layout(size = 1, align = 1);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_ty_path!(["crate", "config", "Foo"], &ty.ty);
}

#[test]
fn cpp_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod c++::a::b {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `c++::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod c++::a::b {
               │         ────┬────  
               │             ╰────── `c++::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn free_fn_rejected_inside_cpp_scope() {
    // There's no real Rust function living in the generated c++::-only
    // module tree, so a top-level `fn` declaration inside `mod c++::a {
    // ... }` must be rejected rather than silently misinterpreting the
    // c++::-only path as if it were a real, absolute Rust path.
    check_fail(
        r#"
mod c++::a {
    fn foo(i32) -> bool;
}
    "#,
        expect![[r#"
            Error: a free function cannot be declared inside a c++:: scope
               ╭─[ test.zng:3:5 ]
               │
             3 │     fn foo(i32) -> bool;
               │     ─────────┬─────────  
               │              ╰─────────── a free function cannot be declared inside a c++:: scope
            ───╯
        "#]],
    );
}

#[test]
fn crate_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod crate::bar {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `crate::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod crate::bar {
               │         ─────┬────  
               │              ╰────── `crate::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn absolute_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod ::std::bar {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod ::std::bar {
               │         ─────┬────  
               │              ╰────── `::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn relative_mod_nested_in_cpp_mod_extends_the_cpp_prefix() {
    // `mod c++::a { mod b { type Name { ... } } }` is `c++::a::b::Name` --
    // a plain relative `mod` nested inside a `c++::` scope composes onto the
    // same `c++::` prefix rather than starting a fresh Rust module path.
    let parsed = parse_str(
        r#"
mod c++::a {
    mod b {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "b", "Name"], &ty.ty);
}

#[test]
fn aliased_type_referenced_inside_cpp_mod_still_resolves_via_the_alias() {
    // A bare relative name that matches an alias must still expand via that
    // alias, even when referenced from inside a `c++::` scope -- it must
    // NOT get the `c++::` prefix composed onto it instead.
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;

mod c++::a {
    type Name {
        #layout(size = 16, align = 8);
        field s (offset = auto, type = MyString);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    // The type's own declaration still composes onto the c++:: prefix:
    assert_cpp(&["a", "Name"], &ty.ty);
    // But the aliased field type resolves as the real Rust path, not
    // `Cpp(["a", "s"])`:
    let field = ty.fields.first().expect("no field parsed");
    assert_ty_path!(["std", "string", "String"], &field.ty);
}

#[test]
fn unprefixed_root_alias_referenced_inside_cpp_mod_keeps_rust_target() {
    let parsed = parse_str(
        r#"
use std::string::String as MyString;

mod c++::a {
    type Name {
        #layout(size = 16, align = 8);
        field s (offset = auto, type = MyString);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "Name"], &ty.ty);
    let field = ty.fields.first().expect("no field parsed");
    assert_ty_path!(["std", "string", "String"], &field.ty);
}

#[test]
fn alias_in_cpp_mod_inherited_into_nested_mod_resolves_against_declaring_mod() {
    let parsed = parse_str(
        r#"
mod c++::a {
    use Foo as MyFoo;
    mod b {
        extern "C++" {
            safe fn make() -> MyFoo;
        }
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let func = parsed
        .spec
        .extern_cpp_funcs
        .first()
        .expect("no extern C++ fn parsed");
    assert_cpp(&["a", "Foo"], &func.output);
}

#[test]
fn same_module_alias_chains_are_order_independent_and_shadow_parent() {
    let parsed = parse_str(
        r#"
use c++::outer as sub;
use ns::Top as MyTop;
use c++::root_ns as ns;

mod c++::a {
    use sub::Foo as MyFoo;
    use b as sub;

    extern "C++" {
        safe fn make_foo() -> MyFoo;
        safe fn make_top() -> MyTop;
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.extern_cpp_funcs.len(), 2);
    assert_cpp(&["a", "b", "Foo"], &parsed.spec.extern_cpp_funcs[0].output);
    assert_cpp(&["root_ns", "Top"], &parsed.spec.extern_cpp_funcs[1].output);

    // Cyclic and duplicate aliases in the same module emit parse errors cleanly without hanging.
    check_fail(
        r#"
use b as a;
use a as b;
extern "C++" {
    safe fn f(&a);
}
"#,
        expect![[r#"
            Error: cyclic `use` alias definition `a`
               ╭─[ test.zng:2:1 ]
               │
             2 │ use b as a;
               │ ─────┬─────  
               │      ╰─────── cyclic `use` alias definition `a`
            ───╯
        "#]],
    );

    check_fail(
        r#"
use c++::outer as sub;
mod a {
    use sub::Inner as sub;
}
"#,
        expect![[r#"
            Error: cyclic `use` alias definition `sub`
               ╭─[ test.zng:4:5 ]
               │
             4 │     use sub::Inner as sub;
               │     ───────────┬──────────  
               │                ╰──────────── cyclic `use` alias definition `sub`
            ───╯
        "#]],
    );

    check_fail(
        r#"
use c++::x as Foo;
use c++::y as Foo;
"#,
        expect![[r#"
            Error: duplicate `use` alias `Foo` in the same module
               ╭─[ test.zng:3:1 ]
               │
             3 │ use c++::y as Foo;
               │ ─────────┬────────  
               │          ╰────────── duplicate `use` alias `Foo` in the same module
               │
               ├─[ test.zng:3:1 ]
               │
             2 │ use c++::x as Foo;
               │ ─────────┬────────  
               │          ╰────────── previously defined here
            ───╯
        "#]],
    );
}

#[test]
fn entity_path_rust_reference_formats_rust_paths_and_rejects_empty_or_cpp() {
    assert_eq!(
        EntityPath::crate_relative(["foo", "Bar"]).rust_reference(),
        Some("crate::foo::Bar".to_owned()),
    );
    assert_eq!(
        EntityPath::rust(["foo", "Bar"]).rust_reference(),
        Some("::foo::Bar".to_owned()),
    );
    assert_eq!(EntityPath::Rust(Vec::new()).rust_reference(), None);
    assert_eq!(EntityPath::Cpp(Vec::new()).rust_reference(), None);
    assert_eq!(EntityPath::cpp(["a", "Foo"]).rust_reference(), None);
}

#[test]
fn cpp_type_capabilities_follow_from_layout_and_trivially_relocatable() {
    let parsed = parse_str(
        r#"
type c++::a::ByValue {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

// A layout without `#trivially_relocatable` is only checked in C++.
type c++::a::NotRelocatable {
    #layout(size = 8, align = 4);
}

// Heap-allocated: Rust owns a C++ heap pointer + destructor.
type c++::a::Heap {
    #heap_allocated;
}

// No layout: Rust only refers to it.
type c++::a::Opaque {
}
"#,
        NullCfg,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty = |name: &str| {
        parsed
            .spec
            .types
            .iter()
            .find(|t| matches!(&t.ty, RustType::Cpp(segs) if segs.last().unwrap() == name))
            .unwrap_or_else(|| panic!("no type {name}"))
    };
    assert_eq!(ty("ByValue").cpp_name().as_deref(), Some("::a::ByValue"));
    assert_eq!(
        ty("ByValue").cpp_by_value_layout(),
        Some(LayoutPolicy::StackAllocated { size: 8, align: 4 })
    );
    assert!(!ty("ByValue").is_cpp_heap_allocated());
    assert_eq!(ty("NotRelocatable").cpp_by_value_layout(), None);
    assert_eq!(
        ty("NotRelocatable").layout,
        Some(LayoutPolicy::StackAllocated { size: 8, align: 4 })
    );
    assert!(!ty("NotRelocatable").is_cpp_heap_allocated());
    assert!(ty("Heap").is_cpp_heap_allocated());
    assert_eq!(ty("Heap").cpp_by_value_layout(), None);
    assert_eq!(ty("Opaque").cpp_by_value_layout(), None);
    assert_eq!(ty("Opaque").layout, None);
    assert!(!ty("Opaque").is_cpp_heap_allocated());
}

#[test]
fn trivially_relocatable_can_be_declared_in_another_file() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    #trivially_relocatable;
}
"#,
        NullCfg,
        &resolver,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert!(ty.cpp_by_value_layout().is_some());
}

#[test]
fn trivially_relocatable_without_layout_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` needs `#layout(size = X, align = Y)`, which Rust needs to hold the C++ type by value
               ╭─[ test.zng:3:5 ]
               │
             3 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` needs `#layout(size = X, align = Y)`, which Rust needs to hold the C++ type by value
            ───╯
        "#]],
    );
}

#[test]
fn trivially_relocatable_on_rust_type_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
               ╭─[ test.zng:4:5 ]
               │
             4 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
            ───╯
        "#]],
    );
}

#[test]
fn removed_cpp_directives_explain_the_new_syntax() {
    check_fail(
        r#"
type crate::A {
    #cpp_stack_owned "::A" (size = 8, align = 4);
}

type crate::B {
    #cpp_ref "::B";
}

type c++::C {
    #cpp_ref;
}

type crate::D {
    #layout(size = 8, align = 8);
    #cpp_heap_allocated "::D";
}

type crate::E {
    #layout(size = 8, align = 8);
    #cpp_value "0" "::E";
}
"#,
        expect![[r#"
            Error: `#cpp_stack_owned` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:3:5 ]
               │
             3 │     #cpp_stack_owned "::A" (size = 8, align = 4);
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { #layout(size = X, align = Y); #trivially_relocatable; }`; Rust holds C++ types by value when they have a layout and are trivially relocatable
            ───╯
            Error: `#cpp_ref` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:7:5 ]
               │
             7 │     #cpp_ref "::B";
               │     ───────┬───────  
               │            ╰───────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { ... }` with no layout; Rust can then only refer to it
            ───╯
            Error: `#cpp_ref` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:11:5 ]
                │
             11 │     #cpp_ref;
                │     ────┬────  
                │         ╰────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout; Rust can then only refer to it
            ────╯
            Error: `#cpp_heap_allocated` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:16:5 ]
                │
             16 │     #cpp_heap_allocated "::D";
                │     ─────────────┬────────────  
                │                  ╰────────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { #heap_allocated; }`
            ────╯
            Error: `#cpp_value` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:21:5 ]
                │
             21 │     #cpp_value "0" "::E";
                │     ──────────┬──────────  
                │               ╰──────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { #heap_allocated; }`
            ────╯
        "#]],
    );

    // Splitting a legacy `#cpp_*` directive from `#layout` across `merge`d files, reopened `type`
    // blocks, or `type<T>` templates must not emit cascading layout conflict or missing-layout
    // errors.
    let resolver = MockFilesystem::new(vec![(
        "./layout.zng",
        r#"
type crate::Split {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let err = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            r#"
merge "./layout.zng";

type crate::Split {
    #cpp_stack_owned "::Split";
}

type crate::Reopened {
    #layout(size = 8, align = 4);
}
type crate::Reopened {
    #cpp_ref "::Reopened";
}

#unstable(template_types)
type<T> crate::Templated<T> {
    #layout(size = 8, align = 8);
}
type crate::Templated<i32> {
    #cpp_stack_owned "::Templated";
}
"#,
            NullCfg,
            &resolver,
            |_| {},
        );
    })
    .unwrap_err()
    .downcast::<ErrorText>()
    .expect("expected ErrorText")
    .0;
    assert!(
        err.contains("`#cpp_stack_owned` is no longer supported"),
        "{err}"
    );
    assert!(err.contains("`#cpp_ref` is no longer supported"), "{err}");
    assert!(
        !err.contains("Conflicting layout policy"),
        "unexpected layout conflict error:\n{err}"
    );
    assert!(
        !err.contains("No layout policy"),
        "unexpected missing layout error:\n{err}"
    );
}

#[test]
fn only_by_ref_and_unsized_on_cpp_type_are_rejected() {
    check_fail(
        r#"
type c++::A {
    #heap_allocated;
    #trivially_relocatable;
}

type c++::B {
    #only_by_ref;
}

type c++::C {
    wellknown_traits(?Sized);
}

type c++::D {
    #layout_conservative(size = 16, align = 8);
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` needs `#layout(size = X, align = Y)`, which Rust needs to hold the C++ type by value
               ╭─[ test.zng:4:5 ]
               │
             4 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` needs `#layout(size = X, align = Y)`, which Rust needs to hold the C++ type by value
            ───╯
            Error: `#only_by_ref` is for Rust types; a `c++::` type without a layout is already reference-only
               ╭─[ test.zng:8:5 ]
               │
             8 │     #only_by_ref;
               │     ──────┬─────  
               │           ╰─────── `#only_by_ref` is for Rust types; a `c++::` type without a layout is already reference-only
            ───╯
            Error: `?Sized` is not supported on C++ types; a `c++::` type without a layout is already reference-only
                ╭─[ test.zng:12:22 ]
                │
             12 │     wellknown_traits(?Sized);
                │                      ───┬──  
                │                         ╰──── `?Sized` is not supported on C++ types; a `c++::` type without a layout is already reference-only
            ────╯
            Error: `#layout_conservative` cannot be used on `c++::` types; use `#layout(size = X, align = Y)` instead
                ╭─[ test.zng:16:5 ]
                │
             16 │     #layout_conservative(size = 16, align = 8);
                │     ─────────────────────┬────────────────────  
                │                          ╰────────────────────── `#layout_conservative` cannot be used on `c++::` types; use `#layout(size = X, align = Y)` instead
            ────╯
        "#]],
    );

    // Also rejected when inherited from `#unstable(template_types)`.
    let r = catch_unwind(|| {
        let _ = parse_str(
            "#unstable(template_types)\ntype<T> T { #only_by_ref; }\ntype c++::Foo {}\n",
            NullCfg,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`#only_by_ref` is for Rust types"),
        "unexpected error:\n{err}"
    );
}

#[test]
fn processed_files_includes_imported_module_and_its_merged_files() {
    let resolver = MockFilesystem::new(vec![
        (
            "./bar.zng",
            "#crate bar_bindings\nmerge \"./bar_part.zng\";\ntype c++::ns::Bar {}\n",
        ),
        ("./bar_part.zng", "type c++::ns::BarPart {}\n"),
    ]);
    let parsed = parse_str_with_resolver("import \"./bar.zng\";\n", NullCfg, &resolver, |_| {});
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(file_names.len(), 3, "got: {file_names:?}");
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"bar.zng"));
    assert!(file_names.contains(&"bar_part.zng"));
}

#[test]
fn crate_directive_sets_the_crate_name_and_allows_identical_duplicates() {
    let parsed = parse_str(
        r#"
#crate my_bindings
#crate my_bindings
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.crate_name.as_deref(), Some("my_bindings"));

    // Valid Rust crate names that happen to be Zngur-only keywords (`merge`, `import`, `safe`).
    for kw_crate in ["merge", "import", "safe"] {
        let parsed = parse_str(&format!("#crate {kw_crate}\n"), NullCfg, |w| {
            panic!("unexpected warning: {w}")
        });
        assert_eq!(parsed.spec.crate_name.as_deref(), Some(kw_crate));
    }
}

#[test]
fn crate_directive_rejects_self_and_super() {
    check_fail(
        r#"
#crate self
#crate super
#crate Self
#crate struct
#crate crate
#crate mod
mod foo {
    #crate bar
}
"#,
        expect![[r#"
            Error: `self` cannot be used as a `#crate` name
               ╭─[ test.zng:2:2 ]
               │
             2 │ #crate self
               │  ─────┬────  
               │       ╰────── `self` cannot be used as a `#crate` name
            ───╯
            Error: `super` cannot be used as a `#crate` name
               ╭─[ test.zng:3:2 ]
               │
             3 │ #crate super
               │  ─────┬─────  
               │       ╰─────── `super` cannot be used as a `#crate` name
            ───╯
            Error: `Self` cannot be used as a `#crate` name
               ╭─[ test.zng:4:2 ]
               │
             4 │ #crate Self
               │  ─────┬────  
               │       ╰────── `Self` cannot be used as a `#crate` name
            ───╯
            Error: `struct` cannot be used as a `#crate` name
               ╭─[ test.zng:5:2 ]
               │
             5 │ #crate struct
               │  ──────┬─────  
               │        ╰─────── `struct` cannot be used as a `#crate` name
            ───╯
            Error: `crate` cannot be used as a `#crate` name
               ╭─[ test.zng:6:2 ]
               │
             6 │ #crate crate
               │  ─────┬─────  
               │       ╰─────── `crate` cannot be used as a `#crate` name
            ───╯
            Error: `mod` cannot be used as a `#crate` name
               ╭─[ test.zng:7:2 ]
               │
             7 │ #crate mod
               │  ────┬────  
               │      ╰────── `mod` cannot be used as a `#crate` name
            ───╯
            Error: `#crate` can only be used at the top level of a file, not inside a `mod` block
               ╭─[ test.zng:9:6 ]
               │
             9 │     #crate bar
               │      ────┬────  
               │          ╰────── `#crate` can only be used at the top level of a file, not inside a `mod` block
            ───╯
        "#]],
    );
}

#[test]
fn crate_directive_not_allowed_in_merged_files() {
    let resolver = MockFilesystem::new(vec![("./merged.zng", "#crate other")]);
    check_import_fail(
        r#"
merge "./merged.zng";
"#,
        expect![[r#"
            Error: `#crate` can only be used in the top-level file of a zngur module, not in merged files
               ╭─[ ./merged.zng:1:2 ]
               │
             1 │ #crate other
               │  ─────┬─────  
               │       ╰─────── `#crate` can only be used in the top-level file of a zngur module, not in merged files
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn conflicting_crate_directives_are_rejected() {
    check_fail(
        r#"
#crate first
#crate second
"#,
        expect![[r#"
            Error: conflicting `#crate`: already set to `first`
               ╭─[ test.zng:3:2 ]
               │
             3 │ #crate second
               │  ──────┬─────  
               │        ╰─────── conflicting `#crate`: already set to `first`
            ───╯
        "#]],
    );
}

#[test]
fn imported_module_cpp_types_and_crate_are_resolved_including_merged_files() {
    let resolver = MockFilesystem::new(vec![
        (
            "./bar.zng",
            r#"
#crate bar_bindings
merge "./bar_extra.zng";

type c++::ns::Bar {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

type crate::RustOnly {
    #layout(size = 1, align = 1);
}
"#,
        ),
        (
            "./bar_extra.zng",
            r#"
type c++::ns::BarExtra {
}
"#,
        ),
    ]);
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

mod crate {
    fn use_bar(&c++::ns::Bar, &c++::ns::BarExtra);
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let [module] = parsed.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(module.crate_name.as_deref(), Some("bar_bindings"));
    assert_eq!(
        module.cpp_types,
        vec![
            vec!["ns".to_owned(), "Bar".to_owned()],
            vec!["ns".to_owned(), "BarExtra".to_owned()],
        ]
    );
    // The imported module's types aren't part of this module's spec.
    assert!(parsed.spec.types.is_empty());
}

#[test]
fn duplicate_module_import_across_merged_files_succeeds() {
    let resolver = MockFilesystem::new(vec![
        ("./bar.zng", "#crate bar_bindings\ntype c++::ns::Bar {}\n"),
        ("./a.zng", "import \"./bar.zng\";\n"),
        ("./b.zng", "import \"./bar.zng\";\n"),
    ]);
    let parsed = parse_str_with_resolver(
        "merge \"./a.zng\";\nmerge \"./b.zng\";\n",
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.imported_modules.len(), 2);
    let bar_count = parsed
        .processed_files
        .iter()
        .filter(|p| p.file_name().and_then(|s| s.to_str()) == Some("bar.zng"))
        .count();
    assert_eq!(
        bar_count, 1,
        "processed_files: {:?}",
        parsed.processed_files
    );
}

#[test]
fn colliding_cpp_types_across_local_and_imported_modules_are_rejected() {
    // 1. Local `c++::` type colliding with imported `c++::` type.
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        "#crate bar_bindings\ntype c++::ns::Bar {}\n",
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./bar.zng\";\ntype c++::ns::Bar {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`c++::ns::Bar` is already imported from `./bar.zng`"),
        "unexpected error:\n{err}"
    );

    // 2. Two imported modules from different crates declaring the same `c++::` type.
    let resolver2 = MockFilesystem::new(vec![
        (
            "./one.zng",
            "#crate one_bindings\ntype c++::ns::Shared {}\n",
        ),
        (
            "./two.zng",
            "#crate two_bindings\ntype c++::ns::Shared {}\n",
        ),
    ]);
    let r2 = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./one.zng\";\nimport \"./two.zng\";\n",
            NullCfg,
            &resolver2,
            |_| {},
        );
    });
    let err2 = r2
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err2.contains(
            "`c++::ns::Shared` is imported from multiple crates (`one_bindings` and `two_bindings`)"
        ),
        "unexpected error:\n{err2}"
    );
}

#[test]
fn imported_module_spans_do_not_pollute_importing_module_diagnostics() {
    // `./bar.zng` declares `crate::Foo` (valid). `test.zng` declares `crate::Foo` WITHOUT a
    // layout: the missing-layout error must point to `test.zng`, never `./bar.zng`.
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        "#crate bar_bindings\ntype crate::Foo { #layout(size = 1, align = 1); }\n",
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./bar.zng\";\ntype crate::Foo {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(err.contains("test.zng"), "missing `test.zng` in:\n{err}");
    assert!(
        !err.contains("bar.zng"),
        "imported module `./bar.zng` polluted spans in:\n{err}"
    );
}

#[test]
fn error_in_imported_module_does_not_cascade_or_abort_sibling_merges() {
    // `./broken.zng` mistakenly puts `#crate` in a merged file `./broken_sub.zng`.
    // 1. We should NOT get a second misleading "needs a `#crate <name>`" error.
    // 2. Sibling `merge "./sibling.zng";` must still be parsed (so an error in `./sibling.zng`
    //    is still reported).
    let resolver = MockFilesystem::new(vec![
        (
            "./broken.zng",
            "merge \"./broken_sub.zng\";\ntype c++::ns::Bar {}\n",
        ),
        ("./broken_sub.zng", "#crate bar_bindings\n"),
        ("./sibling.zng", "type crate::MissingLayout {}\n"),
    ]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./broken.zng\";\nmerge \"./sibling.zng\";\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`#crate` can only be used in the top-level file"),
        "missing `#crate` merged-file error in:\n{err}"
    );
    assert!(
        !err.contains("needs a `#crate <name>` directive"),
        "unexpected cascading missing-`#crate` error in:\n{err}"
    );
    assert!(
        err.contains("sibling.zng"),
        "sibling merge `./sibling.zng` was skipped:\n{err}"
    );
}

#[test]
fn imported_module_with_cpp_types_needs_a_crate() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
type c++::ns::Bar {
}
"#,
    )]);
    check_import_fail(
        r#"
import "./bar.zng";
"#,
        expect![[r#"
            Error: imported module `./bar.zng` declares `c++::` types, so it needs a `#crate <name>` directive for this module to find their Rust wrappers (at `::<name>::generated::...`)
               ╭─[ test.zng:2:1 ]
               │
             2 │ import "./bar.zng";
               │ ─────────┬─────────  
               │          ╰─────────── imported module `./bar.zng` declares `c++::` types, so it needs a `#crate <name>` directive for this module to find their Rust wrappers (at `::<name>::generated::...`)
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn relative_module_imports_inside_merged_subdirectory_are_rebased_onto_root() {
    let resolver = MockFilesystem::new(vec![
        (
            "./sub/part.zng",
            "import \"./bar.zng\";\nimport \"../other/baz.zng\";\nimport \"../../outside/qux.zng\";\nimport \"header_only.zng\";\n",
        ),
        (
            "./sub/bar.zng",
            "#crate bar_bindings\ntype c++::ns::Bar {}\n",
        ),
        (
            "./other/baz.zng",
            "#crate baz_bindings\ntype c++::ns::Baz {}\n",
        ),
        (
            "../outside/qux.zng",
            "#crate qux_bindings\ntype c++::ns::Qux {}\n",
        ),
    ]);
    let parsed = parse_str_with_resolver("merge \"./sub/part.zng\";\n", NullCfg, &resolver, |w| {
        panic!("unexpected warning: {w}")
    });
    assert_eq!(parsed.spec.imported_modules.len(), 4);
    assert_eq!(
        parsed.spec.imported_modules[0].path.to_str(),
        Some("./sub/bar.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[0].crate_name.as_deref(),
        Some("bar_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[0].cpp_types,
        vec![vec!["ns".to_owned(), "Bar".to_owned()]]
    );
    assert_eq!(
        parsed.spec.imported_modules[1].path.to_str(),
        Some("./other/baz.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[1].crate_name.as_deref(),
        Some("baz_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[1].cpp_types,
        vec![vec!["ns".to_owned(), "Baz".to_owned()]]
    );
    assert_eq!(
        parsed.spec.imported_modules[2].path.to_str(),
        Some("../outside/qux.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[2].crate_name.as_deref(),
        Some("qux_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[2].cpp_types,
        vec![vec!["ns".to_owned(), "Qux".to_owned()]]
    );
    // Bare C++ include-search paths without `./` or `../` are preserved as-is.
    assert_eq!(
        parsed.spec.imported_modules[3].path,
        std::path::PathBuf::from("header_only.zng")
    );
    assert_eq!(parsed.spec.imported_modules[3].crate_name, None);
}

#[test]
fn importing_module_with_same_crate_directive_suggests_merge() {
    let resolver = MockFilesystem::new(vec![(
        "./sub.zng",
        "#crate my_crate\ntype c++::ns::Bar {}\n",
    )]);
    check_import_fail(
        r#"
#crate my_crate
import "./sub.zng";
"#,
        expect![[r#"
            Error: imported module `./sub.zng` has the same `#crate` (`my_crate`) as this module; use `merge "./sub.zng";` to split `.zng` files within a single crate
               ╭─[ test.zng:3:1 ]
               │
             3 │ import "./sub.zng";
               │ ─────────┬─────────  
               │          ╰─────────── imported module `./sub.zng` has the same `#crate` (`my_crate`) as this module; use `merge "./sub.zng";` to split `.zng` files within a single crate
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn unreadable_imported_module_is_not_an_error() {
    // `import` only has to name the generated header, which may be found via the C++ include
    // path even if the `.zng` file isn't at the relative path.
    let resolver = MockFilesystem::new(vec![] as Vec<(&str, &str)>);
    let parsed = parse_str_with_resolver(
        r#"
import "elsewhere.zng";
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(parsed.spec.imported_modules[0].crate_name, None);
}
