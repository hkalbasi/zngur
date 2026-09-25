//! This crate contains an API for using the Zngur code generator inside build scripts. For more information
//! about the Zngur itself, see [the documentation](https://hkalbasi.github.io/zngur).

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use zngur_generator::{
    ParseReport, ParsedZngFile, SourceCache, ZngHeaderGenerator, ZngurGenerator,
    cfg::{InMemoryRustCfgProvider, NullCfg, RustCfgProvider},
};

#[must_use]
/// Builder for the Zngur generator.
///
/// Usage:
/// ```ignore
/// let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
/// let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
/// Zngur::from_zng_file(crate_dir.join("main.zng"))
///     .with_cpp_file(out_dir.join("generated.cpp"))
///     .with_h_file(out_dir.join("generated.h"))
///     .with_rs_file(out_dir.join("generated.rs"))
///     .with_depfile(out_dir.join("zngur.d"))
///     .generate();
/// ```
pub struct Zngur {
    zng_file: PathBuf,
    h_file_path: Option<PathBuf>,
    cpp_file_path: Option<PathBuf>,
    rs_file_path: Option<PathBuf>,
    depfile_path: Option<PathBuf>,
    mangling_base: Option<String>,
    cpp_namespace: Option<String>,
    rust_cfg: Option<Box<dyn RustCfgProvider>>,
    zng_header_in_place: bool,
    zng_h_file_path: Option<PathBuf>,
    crate_name: Option<String>,
    warning_sink: Option<Box<dyn Fn(&str)>>,
    report_sink: Option<Box<dyn Fn(bool, &ParseReport, SourceCache)>>,
}

impl Zngur {
    pub fn from_zng_file(zng_file_path: impl AsRef<Path>) -> Self {
        Zngur {
            zng_file: zng_file_path.as_ref().to_owned(),
            h_file_path: None,
            cpp_file_path: None,
            rs_file_path: None,
            depfile_path: None,
            mangling_base: None,
            cpp_namespace: None,
            rust_cfg: None,
            zng_header_in_place: false,
            zng_h_file_path: None,
            crate_name: None,
            warning_sink: None,
            report_sink: None,
        }
    }

    pub fn with_zng_header(mut self, zng_header: impl AsRef<Path>) -> Self {
        self.zng_h_file_path.replace(zng_header.as_ref().to_owned());
        self
    }

    pub fn with_h_file(mut self, path: impl AsRef<Path>) -> Self {
        self.h_file_path = Some(path.as_ref().to_owned());
        self
    }

    pub fn with_cpp_file(mut self, path: impl AsRef<Path>) -> Self {
        self.cpp_file_path = Some(path.as_ref().to_owned());
        self
    }

    pub fn with_rs_file(mut self, path: impl AsRef<Path>) -> Self {
        self.rs_file_path = Some(path.as_ref().to_owned());
        self
    }

    /// Set the path for the dependency file (.d file) output.
    ///
    /// The dependency file lists all .zng files that were processed (main file + imports).
    /// This can be used by build systems to detect when regeneration is needed.
    pub fn with_depfile(mut self, path: impl AsRef<Path>) -> Self {
        self.depfile_path = Some(path.as_ref().to_owned());
        self
    }

    pub fn with_mangling_base(mut self, mangling_base: &str) -> Self {
        self.mangling_base = Some(mangling_base.to_owned());
        self
    }

    pub fn with_cpp_namespace(mut self, cpp_namespace: &str) -> Self {
        self.cpp_namespace = Some(cpp_namespace.to_owned());
        self
    }

    pub fn with_crate_name(mut self, crate_name: &str) -> Self {
        self.crate_name = Some(crate_name.to_owned());
        self
    }

    pub fn with_rust_cargo_cfg(mut self) -> Self {
        self.rust_cfg = Some(Box::new(
            InMemoryRustCfgProvider::default().load_from_cargo_env(),
        ));
        self
    }

    pub fn with_zng_header_in_place_as(mut self, value: bool) -> Self {
        self.zng_header_in_place = value;
        self
    }

    pub fn with_zng_header_in_place(self) -> Self {
        self.with_zng_header_in_place_as(true)
    }

    pub fn with_rust_in_memory_cfg<'a, CfgPairs, CfgKey, CfgValues>(
        mut self,
        cfg_values: CfgPairs,
    ) -> Self
    where
        CfgPairs: IntoIterator<Item = (CfgKey, CfgValues)>,
        CfgKey: AsRef<str> + 'a,
        CfgValues: Clone + IntoIterator + 'a,
        <CfgValues as IntoIterator>::Item: AsRef<str>,
    {
        self.rust_cfg = Some(Box::new(
            InMemoryRustCfgProvider::default().with_values(cfg_values),
        ));
        self
    }

    /// Set how deprecation and other non-fatal warnings produced while parsing the
    /// `.zng` file are reported. Each warning is a rendered (possibly multi-line)
    /// diagnostic, already labeled (e.g. `Warning: ...`), pointing at the relevant
    /// source location.
    ///
    /// By default (if this is never called), warnings are printed to stdout using
    /// Cargo's `cargo:warning=` build-script convention, which is appropriate when
    /// [`generate`](Self::generate) is called from a build script. Callers that are
    /// not running inside a build script (e.g. a CLI) should call this to print
    /// warnings some other way, e.g. `.with_warning_sink(|w| eprintln!("{w}"))`.
    pub fn with_warning_sink(mut self, sink: impl Fn(&str) + 'static) -> Self {
        self.warning_sink = Some(Box::new(sink));
        self
    }

    /// Set how all reports produced while parsing the `.zng` file are handled.
    /// This option overrides the behavior of [`with_warning_sink`](Self::with_warning_sink)
    /// as well as the default behavior of printing fatal reports to stderr.
    ///
    /// The passed function is called once with each report as it arrives.
    ///
    ///   - The first [`bool`] parameter is a flag for if the report is an error and should
    /// invalidate the parse result.
    ///   - The second parameter is an [`ariadne::Report`](zngur_generator::ParseReport)
    ///   using a [`span`](zngur_generator::ReportSpan) which maps a [`SourceId`](zngur_generator::SourceId)
    ///   to a source retrievable from the passed [`SourceCache`].
    ///   - The third parameter is a [`SourceCache`](zngur_generator::SourceCache) used to retrieve
    ///   the full source text by id.
    ///
    /// The passed report can use it's [`write`] or [`write_for_stdout`] methods
    /// to write the report to a buffer and they can be handled from there.
    /// The first method will render console colors and the later will not.
    ///
    /// You could also use the the report's [`eprint`] or [`print`] methods
    /// to print the report directly to stderr or stdout.
    ///
    /// ```rust
    /// # use zngur::Zngur;
    ///
    /// let mut zng = Zngur::from_zng_file("path/to/main.zng")
    ///     .with_report_sink(|fatal, report, sources| {      
    ///         if !fatal {
    ///             // a warning
    ///             let mut buf = Vec::new();
    ///             report.write_for_stdout(sources, &mut buf).unwrap();
    ///             // now the report is stored in a string
    ///             let r = String::from_utf8(buf).unwrap();
    ///         } else {
    ///             // an error, the generate call will fail
    ///             report.eprint(sources).unwrap();
    ///         }
    ///     });
    /// ```
    ///
    /// [`write`]: zngur_generator::ParseReport::write
    /// [`write_for_stdout`]: zngur_generator::ParseReport::write_for_stdout
    /// [`eprint`]: zngur_generator::ParseReport::eprint
    /// [`print`]: zngur_generator::ParseReport::print
    pub fn with_report_sink(
        mut self,
        sink: impl Fn(bool, &ParseReport, SourceCache) + 'static,
    ) -> Self {
        self.report_sink = Some(Box::new(sink));
        self
    }

    pub fn generate(self) {
        match self.try_generate() {
            Ok(()) => {} // no errors
            Err(msg) => {
                // errors were encountered. panic to preserve old behavior
                panic!("{msg}");
            }
        }
    }

    pub fn try_generate(self) -> Result<(), String> {
        let warning_sink = self.warning_sink.unwrap_or_else(|| {
            Box::new(|w: &str| {
                // `cargo:warning=` is a line-based directive, so a multi-line warning
                // needs the prefix repeated on every line to stay visible to Cargo.
                for line in w.lines() {
                    println!("cargo::warning={line}");
                }
            })
        });
        let mut report_sink = self.report_sink.unwrap_or_else(|| {
            Box::new(
                |fatal: bool, report: &ParseReport, source_cache: SourceCache| {
                    if !fatal {
                        // print the report to a string and call the warning sink
                        let mut buf = Vec::new();
                        let _ = report.write_for_stdout(source_cache, &mut buf);
                        let w = String::from_utf8_lossy(&buf);
                        (warning_sink)(&w);
                    } else {
                        let _ = report.eprint(source_cache);
                    }
                },
            )
        });
        let rust_cfg = self.rust_cfg.unwrap_or_else(|| Box::new(NullCfg));
        let parse_result = ParsedZngFile::parse(&self.zng_file, rust_cfg, &mut report_sink);
        if parse_result.errors > 0 {
            return Err(format!(
                "could not parse `{}` due to {} reported error{}; {} warning{} emitted",
                self.zng_file.display(),
                parse_result.errors,
                if parse_result.errors > 0 { "s" } else { "" },
                parse_result.warnings,
                if parse_result.warnings > 0 { "s" } else { "" }
            ));
        }

        let crate_name = self
            .crate_name
            .or_else(|| std::env::var("CARGO_PKG_NAME").ok())
            .unwrap_or_else(|| "crate".to_owned());
        // We will pass crate_name to ZngurGenerator instead of mutating spec.
        let panic_to_exception = parse_result.spec.convert_panic_to_exception.0;
        let mut file = ZngurGenerator::build_from_zng(parse_result.spec, crate_name);

        let rs_file_path = self
            .rs_file_path
            .ok_or_else(|| "No rs file path provided".to_owned())?;
        let h_file_path = self
            .h_file_path
            .ok_or_else(|| "No h file path provided".to_owned())?;

        file.0.cpp_include_header_name = h_file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        if let Some(cpp_namespace) = &self.cpp_namespace {
            file.0.mangling_base = cpp_namespace.clone();
            file.0.cpp_namespace = Some(cpp_namespace.clone());
        }

        if let Some(mangling_base) = self.mangling_base.or_else(|| file.0.cpp_namespace.clone()) {
            // println!("Mangling: {mangling_base}");
            file.0.mangling_base = mangling_base;
        }
        let (rust, h, cpp) = file.render(self.zng_header_in_place);

        File::create(&rs_file_path)
            .map_err(|err| format!("Failed to create file {}: {err}", rs_file_path.display()))?
            .write_all(rust.as_bytes())
            .map_err(|err| format!("Failed to write file {}: {err}", rs_file_path.display()))?;
        File::create(&h_file_path)
            .map_err(|err| format!("Failed to create file {}: {err}", h_file_path.display()))?
            .write_all(h.as_bytes())
            .map_err(|err| format!("Failed to write file {}: {err}", h_file_path.display()))?;
        if let Some(cpp) = &cpp {
            let cpp_file_path = self
                .cpp_file_path
                .as_ref()
                .ok_or_else(|| "No cpp file path provided".to_owned())?;
            File::create(cpp_file_path)
                .map_err(|err| format!("Failed to create file {}: {err}", cpp_file_path.display()))?
                .write_all(cpp.as_bytes())
                .map_err(|err| {
                    format!("Failed to write file {}: {err}", cpp_file_path.display())
                })?;
        }

        // Write dependency file if requested
        if let Some(depfile_path) = self.depfile_path {
            let mut targets = vec![
                h_file_path.display().to_string(),
                rs_file_path.display().to_string(),
            ];
            if let Some(cpp_path) = self.cpp_file_path {
                if cpp.is_some() {
                    targets.push(cpp_path.display().to_string());
                }
            }

            // Format: "target1 target2: dep1 dep2 dep3"
            let deps: Vec<String> = parse_result
                .processed_files
                .iter()
                .map(|p| p.display().to_string())
                .collect();

            let depfile_content = format!("{}: {}\n", targets.join(" "), deps.join(" "));

            File::create(depfile_path)
                .unwrap()
                .write_all(depfile_content.as_bytes())
                .unwrap();
        }
        if let Some(zng_h) = self.zng_h_file_path {
            let mut zng = ZngurHdr::new()
                .with_panic_to_exception_as(panic_to_exception)
                .with_zng_header(zng_h);
            if let Some(cpp_namespace) = &self.cpp_namespace {
                zng = zng.with_cpp_namespace(&cpp_namespace);
            }
            zng.generate()?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ZngurHdr {
    panic_to_exception: bool,
    zng_header_file: Option<PathBuf>,
    cpp_namespace: Option<String>,
}

impl ZngurHdr {
    pub const fn new() -> Self {
        Self {
            panic_to_exception: false,
            zng_header_file: None,
            cpp_namespace: None,
        }
    }

    pub fn with_panic_to_exception(self) -> Self {
        self.with_panic_to_exception_as(true)
    }

    pub fn without_panic_to_exception(self) -> Self {
        self.with_panic_to_exception_as(false)
    }

    pub fn with_panic_to_exception_as(mut self, panic_to_exception: bool) -> Self {
        self.panic_to_exception = panic_to_exception;
        self
    }

    pub fn with_zng_header(mut self, zng_header: impl Into<PathBuf>) -> Self {
        self.zng_header_file.replace(zng_header.into());
        self
    }

    pub fn with_cpp_namespace(mut self, cpp_namespace: &str) -> Self {
        self.cpp_namespace = Some(cpp_namespace.to_owned());
        self
    }

    pub fn generate(self) -> Result<(), String> {
        let generator = ZngHeaderGenerator {
            panic_to_exception: self.panic_to_exception,
            cpp_namespace: self.cpp_namespace.unwrap_or_else(|| "rust".to_owned()),
        };

        let out_h = self
            .zng_header_file
            .ok_or_else(|| "Missing zng header output file".to_owned())?;
        let rendered = generator
            .render()
            .map_err(|_| format!("Failed to generate zngur header "))?;
        std::fs::write(&out_h, rendered)
            .map_err(|_| format!("Couldn't write contents to {}", out_h.display()))?;
        Ok(())
    }
}
