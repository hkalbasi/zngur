use expect_test::{Expect, expect};
use zngur_def::{ZngurSpec, printing::IDLPrinter};
use zngur_parser::{DefaultImportResolver, ImportResolver, ParsedZngFile, cfg::NullCfg};

#[derive(Debug, Default)]
struct ErrorSink {
    error_buffer: Vec<u8>,
}

pub struct ErrorText(pub String);

impl zngur_parser::ReportSink for ErrorSink {
    fn sink_report(
        &mut self,
        report: &zngur_parser::ReportEntry,
        source_cache: zngur_parser::SourceCache,
    ) {
        if report.fatal {
            report
                .report
                .write(source_cache, &mut self.error_buffer)
                .unwrap();
        }
    }
}

impl ErrorSink {
    fn assert_no_errors(&mut self, src: Option<&str>) {
        {
            if !self.error_buffer.is_empty() {
                let errors = String::from_utf8_lossy(&self.error_buffer);
                match src {
                    Some(src) => {
                        panic!(
                            "assertion `no errors` failed parsing:\n\n{src}\n\nErrors:\n\n{errors}",
                        );
                    }
                    None => {
                        panic!("assertion `no errors` failed:\n\n{errors}");
                    }
                }
            }
        }
    }
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

fn check_round_trip(zng: &str, expected_output: Expect) -> (ZngurSpec, ZngurSpec) {
    check_round_trip_with_resolver(zng, None, expected_output)
}

/// parses the input source to a sepc, the reprints it and parses the printed source
/// returns (parsed: Spec, parsed_reprinted: Spec)
fn check_round_trip_with_resolver(
    zng: &str,
    resolver: Option<Box<dyn ImportResolver>>,
    expected_output: Expect,
) -> (ZngurSpec, ZngurSpec) {
    let resolver = resolver.unwrap_or_else(|| Box::new(DefaultImportResolver));
    let parsed = {
        let mut error_sink = ErrorSink::default();
        let ret = ParsedZngFile::parse_str_with_resolver(
            zng,
            "test.zng",
            NullCfg,
            &resolver,
            &mut error_sink,
        );
        error_sink.assert_no_errors(Some(zng));
        ret
    };
    let mut buf = std::io::BufWriter::new(Vec::new());
    let mut printer = IDLPrinter::new(&mut buf);
    printer.write_idl(&parsed.spec).expect("printing failure");
    let bytes = buf.into_inner().expect("buffer flush error");
    let idl = str::from_utf8(&bytes).expect("utf8 error");
    expected_output.assert_eq(idl);
    let reparsed = {
        let mut error_sink = ErrorSink::default();
        let ret = ParsedZngFile::parse_str_with_resolver(
            idl,
            "test.zng",
            NullCfg,
            &resolver,
            &mut error_sink,
        );
        error_sink.assert_no_errors(Some(idl));
        ret
    };

    assert_eq!(parsed.spec.types, reparsed.spec.types);
    assert_eq!(parsed.spec.traits, reparsed.spec.traits);
    assert_eq!(parsed.spec.funcs, reparsed.spec.funcs);
    assert_eq!(parsed.spec.extern_cpp_funcs, reparsed.spec.extern_cpp_funcs);
    assert_eq!(parsed.spec.extern_cpp_impls, reparsed.spec.extern_cpp_impls);

    (parsed.spec, reparsed.spec)
}

#[test]
fn reparse_tuple() {
    let _ = check_round_trip(
        r#"
type (i8, u8) {
    #layout(size = 0, align = 1);
}
    "#,
        expect![[r#"
            type (i8, u8) {
                #layout(size = 0, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_slice() {
    let _ = check_round_trip(
        r#"
type [u8] {
    #layout(size = 0, align = 1);
}
    "#,
        expect![[r#"
            type [u8] {
                #layout(size = 0, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_heap_allocated() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #layout(size = 16, align = 8);
                #cpp_heap_allocated "::osmium::Way";
            }
        "#]],
    );
}
#[test]
fn reparse_cpp_ref() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #cpp_ref "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #cpp_ref "::osmium::Way";
            }
        "#]],
    );
}
#[test]
fn reparse_cpp_stack_owned() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #cpp_stack_owned "::osmium::Way" (size = 16, align = 8);
}
    "#,
        expect![[r#"
            type crate::Way {
                #layout(size = 16, align = 8);
                #cpp_stack_owned "::osmium::Way" (size = 16, align = 8);
            }
        "#]],
    );
}

#[test]
fn reparse_alias() {
    let _ = check_round_trip(
        r#"
use ::std::string::String as MyString;
type MyString {
    #layout(size = 24, align = 8);
}
    "#,
        expect![[r#"
            type ::std::string::String {
                #layout(size = 24, align = 8);
            }
        "#]],
    );
}

#[test]
fn reparse_methods() {
    let _ = check_round_trip(
        r#"
type MyType {
    #layout(size = 24, align = 8);

    fn foo(self, &str);
    fn foo2(&self, &str) ->();
    fn foo3(&mut self, &str);
    fn foo4(&self, &str);
    fn borrow(&self) deref &str;
    fn from_trait(&self, i32) -> bool use MyTrait;

    fn renamed(&self) -> &[u8] as renamed_fn;

    fn other_self(Arc<MyType>, &str);

    fn static(&str) -> MyType;

}
    "#,
        expect![[r#"
            type ::MyType {
                #layout(size = 24, align = 8);

                fn foo(self, &str);
                fn foo2(&self, &str);
                fn foo3(&mut self, &str);
                fn foo4(&self, &str);
                fn borrow(&self) deref &str;
                fn from_trait(&self, i32) -> bool use ::MyTrait;
                fn renamed(&self) -> &[u8] as renamed_fn;
                fn other_self(::Arc<::MyType>, &str);
                fn static(&str) -> ::MyType;
            }
        "#]],
    );
}

#[test]
fn reparse_fields() {
    let _ = check_round_trip(
        r#"
type MyType {
    #layout(size = 24, align = 8);

    field foo (offset = 0, type = &str);
    field bar (offset = 8, type = i32);
    field baz (offset = auto, type = f64);
}
    "#,
        expect![[r#"
            type ::MyType {
                #layout(size = 24, align = 8);

                field foo (offset = 0, type = &str);
                field bar (offset = 8, type = i32);
                field baz (offset = auto, type = f64);
            }
        "#]],
    );
}

#[test]
fn reparse_variants() {
    let _ = check_round_trip(
        r#"
type Option<i32> {
    #layout(size = 8, align = 4);
    variant None { }
    variant Some {
        field 0 (offset = auto, type = i32);
    }
}
        "#,
        expect![[r#"
            type ::Option<i32> {
                #layout(size = 8, align = 4);
                variant None {}
                variant Some {
                    field 0 (offset = auto, type = i32);
                }
            }
        "#]],
    );
}

#[test]
fn reparse_imports() {
    let resolver = MockFilesystem::new(vec![(
        "./relative/path.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let _ = check_round_trip_with_resolver(
        r#"
merge "./relative/path.zng";
type Example {
    #layout(size = 1, align = 1);
}
    "#,
        Some(Box::new(resolver)),
        expect![[r#"
            type ::Example {
                #layout(size = 1, align = 1);
            }

            type ::Imported {
                #layout(size = 1, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_convert_panic_to_exception() {
    let _ = check_round_trip(
        r#"
#convert_panic_to_exception
type A {
    #layout(size = 1, align = 1);
}
        "#,
        expect![[r#"
            #convert_panic_to_exception

            type ::A {
                #layout(size = 1, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_extern_cpp() {
    let _ = check_round_trip(
        r#"
extern "C++" {
    safe fn foo();
    unsafe fn bar();
    impl crate::Foo {
        safe fn s_foo();
        unsafe fn u_foo();
    }
    impl crate::MyTraitByCpp for crate::Foo {
        safe fn s_baz();
        unsafe fn u_baz();
    }
}
        "#,
        expect![[r#"
            extern "C++" {

                safe fn foo();
                unsafe fn bar();

                impl crate::Foo {
                    safe fn s_foo();
                    unsafe fn u_foo();
                }
                impl crate::MyTraitByCpp for crate::Foo {
                    safe fn s_baz();
                    unsafe fn u_baz();
                }

            }
        "#]],
    );
}
