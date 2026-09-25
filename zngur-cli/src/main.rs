use std::{collections::HashMap, path::PathBuf};

use clap::Parser;
use zngur::{Zngur, ZngurHdr};

use crate::cfg_extractor::{CfgFromRustc, cfg_from_rustc};

mod cfg_extractor;

#[derive(Clone)]
struct CfgKey {
    pub key: String,
    pub values: Vec<String>,
}

impl CfgKey {
    fn into_tuple(self) -> (String, Vec<String>) {
        (self.key, self.values)
    }
}

impl<'a> From<&'a str> for CfgKey {
    fn from(s: &'a str) -> Self {
        let (key, values_s) = s.split_once('=').unwrap_or((s, ""));
        let values: Vec<String> = values_s
            .split(',')
            .map(|s| {
                (if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
                    &s[1..s.len() - 1]
                } else {
                    s
                })
                .to_owned()
            })
            .collect();
        CfgKey {
            key: key.to_owned(),
            values,
        }
    }
}

#[derive(Parser)]
#[command(version)]
enum Command {
    #[command(alias = "g")]
    Generate {
        /// Path to the zng file
        path: PathBuf,

        /// Path of the generated C++ file, if it is needed
        ///
        /// Default is {ZNG_FILE_PARENT}/generated.cpp
        #[arg(long)]
        cpp_file: Option<PathBuf>,

        /// Path of the generated header file
        ///
        /// Default is {ZNG_FILE_PARENT}/generated.h
        #[arg(long)]
        h_file: Option<PathBuf>,

        /// Path of the generated Rust file
        ///
        /// Default is {ZNG_FILE_PARENT}/src/generated.rs
        #[arg(long)]
        rs_file: Option<PathBuf>,

        /// Path of the dependency file (.d file) to generate
        ///
        /// The dependency file lists all .zng files that were processed.
        /// This can be used by build systems to detect when regeneration is needed.
        #[arg(long)]
        depfile: Option<PathBuf>,

        /// A unique string which is included in zngur symbols to prevent duplicate
        /// symbols in linker
        ///
        /// Default is "rust"
        #[arg(long)]
        mangling_base: Option<String>,

        /// A rust config value of the form key(=value1(,value2 ...)) to use when
        /// generating the zngur spec.
        /// i.e.  -C target_os=linux -C target_feature=sse,sse2 -C debug_assertions
        ///
        /// see https://doc.rust-lang.org/reference/conditional-compilation.html
        /// for possible values
        ///
        /// combined with any values loaded from rustc (if enabled)
        ///
        /// Default is an empty configuration
        #[arg(long = "cfg", short = 'C')]
        rust_cfg: Vec<CfgKey>,

        /// A feature name to enable when generating the zngur spec
        ///
        /// combined with any values loaded from rustc (if enabled)
        ///
        /// Default is no features
        #[arg(long = "feature", short = 'F')]
        rust_features: Vec<String>,

        #[command(flatten)]
        load_rustc_cfg: CfgFromRustc,

        /// When set, the generator will embed the common Zngur types into the generated header.
        ///
        /// The recommended workflow is to generate a `zngur.h` header with `make-zng-header` which should
        /// be shared between all of your generated modules and leave this flag unset.
        #[arg(long = "in-place", short = 'i')]
        zng_header_in_place: bool,

        /// Set the namespace of the generated C++ types. If not provided it defaults to `rust`
        ///
        /// You may use this to customize overarching namespace for all the
        /// generated types in C++, but if your project spans multiple .zng modules, you
        /// must use a shared namespace
        #[arg(long)]
        cpp_namespace: Option<String>,

        /// Set the crate name to be used in the generated code where `crate` appears.
        ///
        /// If not provided, it tries to read `CARGO_PKG_NAME` and defaults to `crate` if unset.
        #[arg(long)]
        crate_name: Option<String>,
    },
    #[command(alias = "h")]
    /// Generates the zngur.h file that contains shared interop definitions used by all generated zngur bridges.
    MakeZngHeader {
        /// Path to the generated header file
        path: PathBuf,

        /// If set, it will generate the "panic to exceptions" mechanism in the generated header.
        ///
        /// Note that each `zng` module will need to use `#convert_panic_to_exception` in order to fully enable it.
        #[arg(long = "panic-to-exception")]
        convert_panic_to_exception: bool,

        /// Set the namespace of the generated C++ types. If not provided it defaults to `rust`
        ///
        /// You may use this to customize overarching namespace for all the
        /// generated types in C++, but if your project spans multiple .zng modules, you
        /// must use a shared namespace
        #[arg(long)]
        cpp_namespace: Option<String>,
    },
}

fn main() {
    let cmd = Command::parse();
    match cmd {
        Command::Generate {
            path,
            cpp_file,
            h_file,
            rs_file,
            depfile,
            mangling_base,
            rust_cfg,
            rust_features,
            load_rustc_cfg,
            zng_header_in_place,
            cpp_namespace,
            crate_name,
        } => {
            let pp = path.parent().unwrap();
            let cpp_file = cpp_file.unwrap_or_else(|| pp.join("generated.cpp"));
            let h_file = h_file.unwrap_or_else(|| pp.join("generated.h"));
            let rs_file = rs_file.unwrap_or_else(|| pp.join("src/generated.rs"));
            let mut zng = Zngur::from_zng_file(&path)
                .with_cpp_file(cpp_file)
                .with_h_file(h_file)
                .with_rs_file(rs_file)
                .with_zng_header_in_place_as(zng_header_in_place)
                .with_warning_sink(|w| eprintln!("{w}"));
            if let Some(cpp_namespace) = cpp_namespace {
                zng = zng.with_cpp_namespace(&cpp_namespace);
            }
            if let Some(crate_name) = crate_name {
                zng = zng.with_crate_name(&crate_name);
            }

            let mut cfg: HashMap<String, Vec<String>> = HashMap::new();
            if load_rustc_cfg.load_cfg_from_rustc {
                cfg.extend(cfg_from_rustc(load_rustc_cfg, &rust_features));
            }
            if !rust_cfg.is_empty() {
                cfg.extend(rust_cfg.into_iter().map(CfgKey::into_tuple));
            }
            if !rust_features.is_empty() {
                cfg.insert("feature".to_owned(), rust_features);
            }
            if !cfg.is_empty() {
                zng = zng.with_rust_in_memory_cfg(cfg);
            }
            if let Some(depfile) = depfile {
                zng = zng.with_depfile(depfile);
            }
            if let Some(mangling_base) = mangling_base {
                zng = zng.with_mangling_base(&mangling_base);
            }
            zng.generate();
        }
        Command::MakeZngHeader {
            path,
            convert_panic_to_exception,
            cpp_namespace,
        } => {
            let mut hdr = ZngurHdr::new()
                .with_panic_to_exception_as(convert_panic_to_exception)
                .with_zng_header(path);
            if let Some(cpp_namespace) = cpp_namespace {
                hdr = hdr.with_cpp_namespace(&cpp_namespace);
            }
            if let Err(err) = hdr.generate() {
                // panic with err message to preserve old behavior
                panic!("{err}");
            }
        }
    }
}
