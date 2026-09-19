use std::fmt::Write;

use itertools::Itertools;
use sha2::{Digest, Sha256};

use crate::{
    ZngurTrait, ZngurWellknownTrait, ZngurWellknownTraitData,
    cpp::{CppFnSig, CppLayoutPolicy, CppPath, CppTraitDefinition, CppTraitMethod, CppType},
};

use zngur_def::*;

pub trait IntoCpp {
    fn into_cpp(&self, namespace: &str, crate_name: &str) -> CppType;
}

impl IntoCpp for RustPathAndGenerics {
    fn into_cpp(&self, namespace: &str, crate_name: &str) -> CppType {
        let RustPathAndGenerics {
            path,
            generics,
            named_generics,
        } = self;
        let named_generics = named_generics.iter().sorted_by_key(|x| &x.0).map(|x| &x.1);
        CppType {
            path: CppPath::from_rust_path(path, namespace, crate_name),
            generic_args: generics
                .iter()
                .chain(named_generics)
                .map(|x| x.into_cpp(namespace, crate_name))
                .collect(),
            tail: None,
        }
    }
}

impl IntoCpp for RustTrait {
    fn into_cpp(&self, namespace: &str, crate_name: &str) -> CppType {
        match self {
            RustTrait::Normal(pg) => pg.into_cpp(namespace, crate_name),
            RustTrait::Fn {
                name,
                inputs,
                output,
            } => CppType {
                path: CppPath::from(&*format!("{namespace}::{name}")),
                generic_args: inputs
                    .iter()
                    .chain(Some(&**output))
                    .map(|x| x.into_cpp(namespace, crate_name))
                    .collect(),
                tail: None,
            },
        }
    }
}

impl IntoCpp for RustType {
    fn into_cpp(&self, namespace: &str, crate_name: &str) -> CppType {
        fn for_builtin(this: &RustType, namespace: &str, crate_name: &str) -> Option<CppType> {
            match this {
                RustType::Primitive(s) => match s {
                    PrimitiveRustType::Uint(s) => Some(CppType::from(&*format!("uint{s}_t"))),
                    PrimitiveRustType::Int(s) => Some(CppType::from(&*format!("int{s}_t"))),
                    PrimitiveRustType::Float(32) => Some(CppType::from("float_t")),
                    PrimitiveRustType::Float(64) => Some(CppType::from("double_t")),
                    PrimitiveRustType::Float(_) => unreachable!(),
                    PrimitiveRustType::Usize => Some(CppType::from("size_t")),
                    PrimitiveRustType::Bool | PrimitiveRustType::Str | PrimitiveRustType::Char => {
                        None
                    }
                },
                RustType::Raw(Mutability::Mut, t) => Some(CppType::from(&*format!(
                    "{}*",
                    for_builtin(t, namespace, crate_name)?
                        .to_string()
                        .strip_prefix("::")?
                ))),
                RustType::Raw(Mutability::Not, t) => Some(CppType::from(&*format!(
                    "{} const*",
                    for_builtin(t, namespace, crate_name)?
                        .to_string()
                        .strip_prefix("::")?
                ))),
                _ => None,
            }
        }
        if let Some(builtin) = for_builtin(self, namespace, crate_name) {
            return builtin;
        }
        match self {
            RustType::Primitive(s) => match s {
                PrimitiveRustType::Bool => CppType::from(&*format!("{namespace}::Bool")),
                PrimitiveRustType::Str => CppType::from(&*format!("{namespace}::Str")),
                PrimitiveRustType::Char => CppType::from(&*format!("{namespace}::Char")),
                _ => unreachable!(),
            },
            RustType::Boxed(t) => CppType {
                path: CppPath::from(&*format!("{namespace}::Box")),
                generic_args: vec![t.into_cpp(namespace, crate_name)],
                tail: None,
            },
            RustType::Ref(m, t) => CppType {
                path: match m {
                    Mutability::Mut => CppPath::from(&*format!("{}::RefMut", namespace)),
                    Mutability::Not => CppPath::from(&*format!("{}::Ref", namespace)),
                },
                generic_args: vec![t.into_cpp(namespace, crate_name)],
                tail: None,
            },
            RustType::Slice(s) => CppType {
                path: CppPath::from(&*format!("{namespace}::Slice")),
                generic_args: vec![s.into_cpp(namespace, crate_name)],
                tail: None,
            },
            RustType::Raw(m, t) => CppType {
                path: match m {
                    Mutability::Mut => CppPath::from(&*format!("{namespace}::RawMut")),
                    Mutability::Not => CppPath::from(&*format!("{namespace}::Raw")),
                },
                generic_args: vec![t.into_cpp(namespace, crate_name)],
                tail: None,
            },
            RustType::Adt(pg) => pg.into_cpp(namespace, crate_name),
            RustType::Cpp(segs) => RustPathAndGenerics {
                path: segs.clone(),
                generics: vec![],
                named_generics: vec![],
            }
            .into_cpp(namespace, crate_name),
            RustType::Tuple(v) => {
                if v.is_empty() {
                    return CppType::from(&*format!("{namespace}::Unit"));
                }
                CppType {
                    path: CppPath::from(&*format!("{namespace}::Tuple")),
                    generic_args: v
                        .into_iter()
                        .map(|x| x.into_cpp(namespace, crate_name))
                        .collect(),
                    tail: None,
                }
            }
            RustType::Dyn(tr, marker_bounds) => {
                let tr_as_cpp_type = tr.into_cpp(namespace, crate_name);
                CppType {
                    path: CppPath::from(&*format!("{namespace}::Dyn")),
                    generic_args: [tr_as_cpp_type]
                        .into_iter()
                        .chain(
                            marker_bounds
                                .iter()
                                .map(|x| CppType::from(&*format!("{namespace}::{x}"))),
                        )
                        .collect(),
                    tail: None,
                }
            }
            RustType::Impl(_, _) => panic!("impl Trait is invalid in C++"),
            RustType::TypeVar(_) => {
                unreachable!("should not attempt to generate definition for unbound TypeVar")
            }
        }
    }
}

pub struct RustFile {
    pub text: String,
    pub panic_to_exception: bool,
    pub mangling_base: String,
}

impl RustFile {
    pub fn new(mangling_base: &str) -> Self {
        Self {
            text: r#"
macro_rules! __zngur_str_as_array {
    ($s:expr) => {{
        const VAL: &str = $s;
        // SAFETY: `VAL` has at least size `N` because it's const len is right there.
        const ARR: [u8; VAL.len()] = unsafe { *(VAL.as_bytes() as *const [u8]).cast() };
        ARR
    }};
}

pub const fn __zngur_usize_num_digits(val: usize) -> usize {
    if val == 0 { 1 } else { val.ilog10() as usize + 1 }
}

pub const fn __zngur_usize_digit(val: usize, digit: usize) -> u8 {
    let mut temp = val;
    let mut i = 0;
    while i < digit {
        temp /= 10;
        i += 1;
    }
    if temp == 0 && val > 0 {
        ::core::panic!("no such digit!")
    } else {
        (temp % 10) as u8
    }
}

pub const fn __zngur_digit_to_ascii(digit: u8) -> u8 {
    ::core::assert!(digit <= 9);
    digit + b'0'
}

pub const fn __zngur_usize_to_digit_array<const N: usize>(val: usize) -> [u8; N] {
    let mut arr: [u8; N] = [0; N];
    let mut i = 0;
    while i < N {
        arr[N - 1 - i] = __zngur_digit_to_ascii(__zngur_usize_digit(val, i));
        i += 1;
    }
    arr
}

macro_rules! __zngur_usize_to_str {
    ($x:expr) => {{
        const VAL: usize = $x;
        const ARR: [u8; __zngur_usize_num_digits(VAL)] = __zngur_usize_to_digit_array(VAL);
        // SAFETY: `ARR` is an ascii byte array which is utf8 compliant
        const STR: &str = unsafe { ::core::str::from_utf8_unchecked(&ARR) };
        STR
    }};
}

pub const fn __zngur_const_str_sum_lengths(strs: &'static [&'static str]) -> usize {
    let ::core::ops::Range { mut start, end } = 0..strs.len();
    let mut sum = 0usize;
    while start < end {
        let i = start;
        start += 1;
        sum += strs[i].len();
    }
    sum
}

pub const fn __zngur_const_concat_strs<const N: usize>(strs: &'static [&'static str]) -> [u8; N] {
    let mut arr = [0u8; N];
    let mut arr_i = 0usize;
    let ::core::ops::Range { mut start, end } = 0..strs.len();
    while start < end {
        let i = start;
        start += 1;
        let slice = strs[i].as_bytes();
        {
            let ::core::ops::Range { mut start, end } = 0..slice.len();
            while start < end {
                let j = start;
                start += 1;
                arr[arr_i] = slice[j];
                arr_i += 1;
            }
        }
    }
    arr
}

macro_rules! __zngur_const_str_concat {
    ( $x:expr, $($rest:expr),+ $(,)?  ) => {{
        const STRS: &'static [&'static str] = & [ $x, $($rest),+ ];
        const LEN: usize = __zngur_const_str_sum_lengths(STRS);
        const ARR: [u8; LEN] = __zngur_const_concat_strs(STRS);
        // SAFETY: `ARR` is an concatenated utf8 byte array built from validated `const &'static str`
        const STR: &str =  unsafe { ::core::str::from_utf8_unchecked(&ARR) };
        STR
    }};
}

macro_rules! __zngur_assert_is_copy {
    ($x:ty $(,)?) => {
        const _: () = {
            const fn static_assert_is_copy<T: Copy>() {}
            static_assert_is_copy::<$x>();
        };
    };
}

macro_rules! __zngur_assert_size {
    ($x:ty, $size:expr $(,)?) => {
        const _: () = ::core::assert!(
            $size == ::core::mem::size_of::<$x>(),
            "{}",
            __zngur_const_str_concat!(
                "zngur declared size of ",
                stringify!($x),
                " is incorrect: expected ",
                __zngur_usize_to_str!($size),
                " , real size is ",
                __zngur_usize_to_str!(::core::mem::size_of::<$x>()),
            )
        );
    };
}

macro_rules! __zngur_assert_align {
    ($x:ty, $align:expr $(,)?) => {
        const _: () = ::core::assert!(
            $align == ::core::mem::align_of::<$x>(),
            "{}",
            __zngur_const_str_concat!(
                "zngur declared align of ",
                stringify!($x),
                " is incorrect: expected ",
                __zngur_usize_to_str!($align),
                " , real align is ",
                __zngur_usize_to_str!(::core::mem::align_of::<$x>()),
            )
        );
    };
}

macro_rules! __zngur_assert_size_conservative {
    ($x:ty, $size:expr $(,)?) => {
        const _: () = ::core::assert!(
            $size >= ::core::mem::size_of::<$x>(),
            "{}",
            __zngur_const_str_concat!(
                "zngur declared conservative size of ",
                stringify!($x),
                " is incorrect: expected size less than or equal to ",
                __zngur_usize_to_str!($size),
                " , real size is ",
                __zngur_usize_to_str!(::core::mem::size_of::<$x>()),
            )
        );
    };
}

macro_rules! __zngur_assert_align_conservative {
    ($x:ty, $align:expr $(,)?) => {
        const _: () = ::core::assert!(
            $align >= ::core::mem::align_of::<$x>(),
            "{}",
            __zngur_const_str_concat!(
                "zngur declared conservative align of ",
                stringify!($x),
                " is incorrect: expected align less than or equal to ",
                __zngur_usize_to_str!($align),
                " , real align is ",
                __zngur_usize_to_str!(::core::mem::align_of::<$x>()),
            )
        );
    };
}

macro_rules! __zngur_assert_has_field {
    ($x:ty, $y:ty, $($field:tt)+ $(,)?) => {
        const _: () = {
            #[allow(dead_code)]
            #[allow(mismatched_lifetime_syntaxes)]
            fn check_field(value: $x) -> $y {
                value.$($field)+
            }
        };
    };
}

macro_rules! __zngur_assert_field_offset {
    ($x:ty, $offset:expr, $($field:tt)+ $(,)?) => {
        const _: () = ::core::assert!(
            $offset == ::core::mem::offset_of!($x, $($field)+),
            "{}",
            __zngur_const_str_concat!(
                "zngur declared offset of field ",
                stringify!($($field)+),
                " in ",
                stringify!($x),
                " is incorrect: expected offset of ",
                __zngur_usize_to_str!($offset),
                " , real offset is ",
                __zngur_usize_to_str!(::core::mem::offset_of!($x, $($field)+)),
            )
        );
    };
}
"#
            .to_owned(),
            panic_to_exception: false,
            mangling_base: mangling_base.to_owned(),
        }
    }
}

impl Write for RustFile {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.text.write_str(s)
    }
}

macro_rules! w {
    ($dst:expr, $($arg:tt)*) => {
        { let _ = write!($dst, $($arg)*); }
    };
}

macro_rules! wln {
    ($dst:expr, $($arg:tt)*) => {
        { let _ = writeln!($dst, $($arg)*); }
    };
}

pub fn hash_of_sig(sig: &[RustType]) -> String {
    let mut text = "".to_owned();
    for elem in sig {
        text += &format!("{elem}+");
    }

    let digset = Sha256::digest(&text);
    hex::encode(&digset[..5])
}

fn mangle_name(name: &str, mangling_base: &str) -> String {
    let mut name = "_zngur_"
        .chars()
        .chain(mangling_base.chars())
        .chain(name.chars().filter(|c| !c.is_whitespace()))
        .chain(Some('_'))
        .collect::<String>();
    let bads = [
        (1, "::<", 'm'),
        (1, ">::", 'n'),
        (1, "->", 'a'),
        (2, "&", 'r'),
        (2, "=", 'e'),
        (2, "<", 'x'),
        (2, ">", 'y'),
        (2, "[", 'j'),
        (2, "]", 'k'),
        (2, "::", 's'),
        (2, ",", 'c'),
        (2, "+", 'l'),
        (2, "(", 'p'),
        (2, ")", 'q'),
        (2, "@", 'z'),
        (2, "-", 'h'),
    ];
    while let Some((pos, which)) = bads.iter().filter_map(|x| Some((name.find(x.1)?, x))).min() {
        name.replace_range(pos..pos + which.1.len(), "_");
        w!(name, "{}{pos}", which.2);
    }
    name
}

/// If `ty` is `Box<dyn ::std::future::Future<Output = T>>` (or `::core::future::Future`), returns `(T, is_send_sync)`.
pub(crate) fn future_output_type(ty: &RustType) -> Option<(&RustType, bool)> {
    let RustType::Boxed(inner) = ty else {
        return None;
    };
    let RustType::Dyn(RustTrait::Normal(pg), bounds) = inner.as_ref() else {
        return None;
    };
    // Accept both `std::future::Future` and `core::future::Future` (and possible leading `::`).
    let path = &pg.path;
    if path.len() < 3 {
        return None;
    }
    if !(path[path.len() - 2] == "future" && path[path.len() - 1] == "Future") {
        return None;
    }
    let crate_seg = &path[path.len() - 3];
    if crate_seg != "std" && crate_seg != "core" {
        return None;
    }
    let output = pg
        .named_generics
        .iter()
        .find(|(name, _)| name == "Output")
        .map(|(_, ty)| ty)?;
    let is_send_sync = bounds.contains(&"Send".to_string()) && bounds.contains(&"Sync".to_string());
    Some((output, is_send_sync))
}

/// Per-output-type symbol names for the coroutine support of a
/// `Box<dyn ::std::future::Future<Output = T>>` declared type.
#[derive(Debug, Clone)]
pub struct CoroFutureShim {
    pub rust_output: RustType,
    pub cpp_output: CppType,
    pub is_send_sync: bool,
    /// Include guard for the C++ shims of this output type. Derived only from
    /// the type (not the crate), so that two crates declaring the same future
    /// type can be included in one C++ translation unit without redefinition.
    pub guard: String,
    /// Rust-provided: polls a `Box<dyn Future<Output = T>>`.
    pub poll_future_fn: String,
    /// Rust-provided: wraps a C++ coroutine handle in a `Box<dyn Future<Output = T>>`.
    pub make_coro_future_fn: String,
    /// C++-provided: drives the C++ coroutine (polls the pending future,
    /// resumes when ready) and extracts the result.
    pub coro_poll_fn: String,
    /// C++-provided: destroys the C++ coroutine handle.
    pub coro_destroy_fn: String,
}

/// Coroutine support for the declared `Box<dyn ::std::future::Future<Output = T>>`
/// types, or `None` if the spec declares no such type.
#[derive(Debug, Clone)]
pub struct CoroSupport {
    pub clone_waker_fn: String,
    pub waker_wake_fn: String,
    pub waker_drop_fn: String,
    pub shims: Vec<CoroFutureShim>,
}

impl CoroSupport {
    pub fn from_types<'a>(
        types: impl IntoIterator<Item = &'a RustType>,
        mangling_base: &str,
        namespace: &str,
        crate_name: &str,
    ) -> Option<Self> {
        let mut shims: Vec<CoroFutureShim> = vec![];
        for ty in types {
            let Some((output, is_send_sync)) = future_output_type(ty) else {
                continue;
            };
            if shims
                .iter()
                .any(|s| &s.rust_output == output && s.is_send_sync == is_send_sync)
            {
                continue;
            }
            let (guard, suffix) = Self::shim_names(output, is_send_sync);
            shims.push(CoroFutureShim {
                rust_output: output.clone(),
                cpp_output: output.into_cpp(namespace, crate_name),
                is_send_sync,
                guard,
                poll_future_fn: mangle_name(&format!("coro_poll_future_{suffix}"), mangling_base),
                make_coro_future_fn: mangle_name(
                    &format!("coro_make_future_{suffix}"),
                    mangling_base,
                ),
                coro_poll_fn: mangle_name(&format!("coro_poll_coro_{suffix}"), mangling_base),
                coro_destroy_fn: mangle_name(&format!("coro_destroy_coro_{suffix}"), mangling_base),
            });
        }
        if shims.is_empty() {
            return None;
        }
        Some(CoroSupport {
            clone_waker_fn: mangle_name("coro_clone_waker", mangling_base),
            waker_wake_fn: mangle_name("coro_waker_wake", mangling_base),
            waker_drop_fn: mangle_name("coro_waker_drop", mangling_base),
            shims,
        })
    }

    fn shim_names(output: &RustType, is_send_sync: bool) -> (String, String) {
        let hash = hash_of_sig(std::slice::from_ref(output));
        let guard = if is_send_sync {
            format!("ZNGUR_CORO_SHIMS_{hash}_SEND_SYNC")
        } else {
            format!("ZNGUR_CORO_SHIMS_{hash}")
        };
        let suffix = if is_send_sync {
            format!("{hash}_send_sync")
        } else {
            hash
        };
        (guard, suffix)
    }
}

impl RustFile {
    fn mangle_name(&self, name: &str) -> String {
        mangle_name(name, &self.mangling_base)
    }

    fn call_cpp_function(&mut self, name: &str, inputs: usize) {
        for n in 0..inputs {
            wln!(self, "let mut i{n} = ::core::mem::MaybeUninit::new(i{n});")
        }
        wln!(self, "let mut r = ::core::mem::MaybeUninit::uninit();");
        w!(self, "{name}");
        for n in 0..inputs {
            w!(self, "i{n}.as_mut_ptr() as *mut u8, ");
        }
        wln!(self, "r.as_mut_ptr() as *mut u8);");
        wln!(self, "r.assume_init()");
    }

    pub fn add_static_is_copy_assert(&mut self, ty: &RustType) {
        wln!(self, r#"__zngur_assert_is_copy!({ty});"#);
    }

    pub fn add_static_size_assert(&mut self, ty: &RustType, size: usize) {
        wln!(self, r#"__zngur_assert_size!({ty}, {size});"#);
    }

    pub fn add_static_align_assert(&mut self, ty: &RustType, align: usize) {
        wln!(self, r#"__zngur_assert_align!({ty}, {align});"#);
    }

    pub fn add_static_size_upper_bound_assert(&mut self, ty: &RustType, size: usize) {
        wln!(self, r#"__zngur_assert_size_conservative!({ty}, {size});"#);
    }

    pub fn add_static_align_upper_bound_assert(&mut self, ty: &RustType, align: usize) {
        wln!(
            self,
            r#"__zngur_assert_align_conservative!({ty}, {align});"#
        );
    }

    pub(crate) fn add_builder_for_dyn_trait(
        &mut self,
        tr: &ZngurTrait,
        namespace: &str,
        crate_name: &str,
    ) -> CppTraitDefinition {
        assert!(matches!(tr.tr, RustTrait::Normal { .. }));
        let mut method_mangled_name = vec![];
        wln!(self, r#"unsafe extern "C" {{"#);
        for method in &tr.methods {
            let name = self.mangle_name(&tr.tr.to_string())
                + "_"
                + &method.name
                + "_"
                + &hash_of_sig(&method.generics)
                + "_"
                + &hash_of_sig(&method.inputs);
            wln!(
                self,
                r#"fn {name}(data: *mut u8, {} o: *mut u8);"#,
                method
                    .inputs
                    .iter()
                    .enumerate()
                    .map(|(n, _)| format!("i{n}: *mut u8,"))
                    .join(" ")
            );
            method_mangled_name.push(name);
        }
        wln!(self, "}}");
        let link_name = self.add_builder_for_dyn_trait_owned(tr, &method_mangled_name);
        let link_name_ref = self.add_builder_for_dyn_trait_borrowed(tr, &method_mangled_name);
        CppTraitDefinition::Normal {
            as_ty: tr.tr.into_cpp(namespace, crate_name),
            methods: tr
                .methods
                .clone()
                .into_iter()
                .zip(method_mangled_name)
                .map(|(x, rust_link_name)| CppTraitMethod {
                    name: x.name,
                    rust_link_name,
                    inputs: x
                        .inputs
                        .into_iter()
                        .map(|x| x.into_cpp(namespace, crate_name))
                        .collect(),
                    output: x.output.into_cpp(namespace, crate_name),
                })
                .collect(),
            link_name,
            link_name_ref,
        }
    }

    fn add_builder_for_dyn_trait_owned(
        &mut self,
        tr: &ZngurTrait,
        method_mangled_name: &[String],
    ) -> String {
        let trait_name = tr.tr.to_string();
        let (trait_without_assocs, assocs) = tr.tr.clone().take_assocs();
        let mangled_name = self.mangle_name(&trait_name);
        wln!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {mangled_name}(
    data: *mut u8,
    destructor: extern "C" fn(*mut u8),
    o: *mut u8,
) {{
    struct Wrapper {{ 
        data: *mut u8,
        destructor: extern "C" fn(*mut u8),
    }}
    impl Drop for Wrapper {{
        fn drop(&mut self) {{
            (self.destructor)(self.data)
        }}
    }}
    impl {trait_without_assocs} for Wrapper {{
"#
        );
        for (name, ty) in assocs {
            wln!(self, "        type {name} = {ty};");
        }
        for (method, rust_link_name) in tr.methods.iter().zip(method_mangled_name) {
            w!(self, "        fn {}(", method.name);
            match method.receiver {
                crate::ZngurMethodReceiver::Static => {
                    panic!("traits with static methods are not object safe");
                }
                crate::ZngurMethodReceiver::Ref(Mutability::Not) => w!(self, "&self"),
                crate::ZngurMethodReceiver::Ref(Mutability::Mut) => w!(self, "&mut self"),
                crate::ZngurMethodReceiver::Move => w!(self, "self"),
            }
            for (i, ty) in method.inputs.iter().enumerate() {
                w!(self, ", i{i}: {ty}");
            }
            wln!(self, ") -> {} {{ unsafe {{", method.output);
            wln!(self, "            let data = self.data;");
            self.call_cpp_function(&format!("{rust_link_name}(data, "), method.inputs.len());
            wln!(self, "        }} }}");
        }
        wln!(
            self,
            r#"
    }}
    unsafe {{ 
        let this = Wrapper {{
            data,
            destructor,
        }};
        let r: Box<dyn {trait_name}> = Box::new(this);
        std::ptr::write(o as *mut _, r)
    }}
}}"#
        );
        mangled_name
    }

    fn add_builder_for_dyn_trait_borrowed(
        &mut self,
        tr: &ZngurTrait,
        method_mangled_name: &[String],
    ) -> String {
        let trait_name = tr.tr.to_string();
        let (trait_without_assocs, assocs) = tr.tr.clone().take_assocs();
        let mangled_name = self.mangle_name(&trait_name) + "_borrowed";
        wln!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {mangled_name}(
    data: *mut u8,
    o: *mut u8,
) {{
    struct Wrapper(());
    impl {trait_without_assocs} for Wrapper {{
"#
        );
        for (name, ty) in assocs {
            wln!(self, "        type {name} = {ty};");
        }
        for (method, rust_link_name) in tr.methods.iter().zip(method_mangled_name) {
            w!(self, "        fn {}(", method.name);
            match method.receiver {
                crate::ZngurMethodReceiver::Static => {
                    panic!("traits with static methods are not object safe");
                }
                crate::ZngurMethodReceiver::Ref(Mutability::Not) => w!(self, "&self"),
                crate::ZngurMethodReceiver::Ref(Mutability::Mut) => w!(self, "&mut self"),
                crate::ZngurMethodReceiver::Move => w!(self, "self"),
            }
            for (i, ty) in method.inputs.iter().enumerate() {
                w!(self, ", i{i}: {ty}");
            }
            wln!(self, ") -> {} {{ unsafe {{", method.output);
            wln!(
                self,
                "            let data = ::std::mem::transmute::<_, *mut u8>(self);"
            );
            self.call_cpp_function(&format!("{rust_link_name}(data, "), method.inputs.len());
            wln!(self, "        }} }}");
        }
        wln!(
            self,
            r#"
    }}
    unsafe {{ 
        let this = data as *mut Wrapper;
        let r: &dyn {trait_name} = &*this;
        std::ptr::write(o as *mut _, r)
    }}
}}"#
        );
        mangled_name
    }

    pub fn add_builder_for_dyn_fn(
        &mut self,
        name: &str,
        inputs: &[RustType],
        output: &RustType,
    ) -> String {
        let mangled_name = self.mangle_name(&inputs.iter().chain(Some(output)).join(", "));
        let trait_str = format!("{name}({}) -> {output}", inputs.iter().join(", "));
        wln!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {mangled_name}(
    data: *mut u8,
    destructor: extern "C" fn(*mut u8),
    call: extern "C" fn(data: *mut u8, {} o: *mut u8),
    o: *mut u8,
) {{
    struct ClosureData {{
        data: *mut u8,
        destructor: extern "C" fn(*mut u8),
    }}
    impl Drop for ClosureData {{
        fn drop(&mut self) {{
            (self.destructor)(self.data)
        }}
    }}
    let this = ClosureData {{ data, destructor }};
    let r: Box<dyn {trait_str}> = Box::new(move |{}| unsafe {{
        _ = &this;
        let data = this.data;
"#,
            inputs
                .iter()
                .enumerate()
                .map(|(n, _)| format!("i{n}: *mut u8, "))
                .join(" "),
            inputs
                .iter()
                .enumerate()
                .map(|(n, ty)| format!("i{n}: {ty}"))
                .join(", "),
        );
        self.call_cpp_function("call(data, ", inputs.len());
        wln!(
            self,
            r#"
    }});
    unsafe {{ std::ptr::write(o as *mut _, r) }}
}}"#
        );
        mangled_name
    }

    pub fn add_tuple_constructor(&mut self, fields: &[RustType]) -> String {
        let constructor = self.mangle_name(&fields.iter().join("&"));
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {constructor}("#
        );
        for name in 0..fields.len() {
            w!(self, "f_{name}: *mut u8, ");
        }
        w!(
            self,
            r#"o: *mut u8) {{ unsafe {{
    ::std::ptr::write(o as *mut _, ("#
        );
        for (name, ty) in fields.iter().enumerate() {
            w!(self, "::std::ptr::read(f_{name} as *mut {ty}), ");
        }
        wln!(self, ")) }} }}");
        constructor
    }

    pub fn add_constructor<'a>(
        &mut self,
        rust_name: &str,
        args: impl IntoIterator<Item = (&'a String, &'a RustType)> + Clone,
    ) -> String {
        let constructor = self.mangle_name(rust_name);
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {constructor}("#
        );
        for (name, _) in args.clone() {
            w!(self, "f_{name}: *mut u8, ");
        }
        w!(
            self,
            r#"o: *mut u8) {{ unsafe {{
    ::std::ptr::write(o as *mut _, {rust_name} {{ "#
        );
        for (name, ty) in args {
            w!(self, "{name}: ::std::ptr::read(f_{name} as *mut {ty}), ");
        }
        wln!(self, "}}) }} }}");
        constructor
    }

    pub(crate) fn add_match_check(&mut self, rust_name: &str) -> String {
        let match_check = self.mangle_name(&format!("{rust_name}_check"));
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {match_check}(i: *mut u8, o: *mut u8) {{ unsafe {{
    *o = matches!(&*(i as *mut &_), {rust_name} {{ .. }}) as u8;
}} }}"#
        );
        match_check
    }

    pub(crate) fn add_discriminant(
        &mut self,
        rust_name: &str,
        variants: &[ZngurVariant],
    ) -> Option<String> {
        if variants.is_empty() {
            return None;
        }

        let name = self.mangle_name(&format!("{rust_name}::match"));
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {name}(i: *mut u8) -> u32 {{ unsafe {{
    match &*(i as *mut {rust_name} as *const _) {{"#
        );
        for (n, variant) in variants.iter().enumerate() {
            let name = &variant.name;
            w!(
                self,
                r#"
        {rust_name}::{name} {{ .. }} => {n},
        "#
            );
        }
        w!(
            self,
            r#"
    }}
}} }}
            "#
        );
        Some(name)
    }

    pub(crate) fn add_field_assertions(
        &mut self,
        field: &ZngurField,
        owner: &RustType,
    ) -> Option<String> {
        let ZngurField { name, ty, offset } = field;
        wln!(self, r#"__zngur_assert_has_field!({owner}, {ty}, {name});"#);
        if let Some(offset) = offset {
            wln!(
                self,
                r#"__zngur_assert_field_offset!({owner}, {offset}, {name});"#
            );
            None
        } else {
            let mn = self.mangle_name(&format!("{}_field_{}_offset", &owner, &name));
            wln!(
                self,
                r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub static {mn}: usize = ::std::mem::offset_of!({owner}, {name});
                "#
            );
            Some(mn)
        }
    }

    pub(crate) fn add_variant_field_calculations(
        &mut self,
        field: &ZngurField,
        owner: &RustType,
        variant: &str,
    ) -> String {
        let ZngurField { name, .. } = field;
        let mn = self.mangle_name(&format!("{owner}_{variant}_field_{name}_offset"));
        // SAFETY: this function is only called from the variant class methods.
        // The only way to obtain variant classes is to match, so it is impossible
        // to obtain an instance of variant class set to a wrong variant,
        // so this match will always pass.
        wln!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {mn}(i: *const u8) -> usize {{ unsafe {{
    let base = &*(i as *const {owner});
    match base {{
        {owner}::{variant} {{ {name}: f, .. }} => {{
            (f as *const _ as usize) - (base as *const _ as usize)
        }}
        _ => std::hint::unreachable_unchecked(),
    }}
}} }}
            "#
        );
        mn
    }

    pub fn add_extern_cpp_impl(
        &mut self,
        owner: &RustType,
        tr: Option<&RustTrait>,
        methods: &[ZngurMethod],
    ) -> Vec<String> {
        let mut mangled_names = vec![];
        w!(self, r#"unsafe extern "C" {{"#);
        for method in methods {
            let mn = self.mangle_name(&format!("{}_extern_method_{}", owner, method.name));
            w!(
                self,
                r#"
    fn {mn}("#
            );
            let input_offset = if method.receiver == ZngurMethodReceiver::Static {
                0
            } else {
                1
            };
            for n in 0..method.inputs.len() + input_offset {
                w!(self, "i{n}: *mut u8, ");
            }
            wln!(self, r#"o: *mut u8);"#);
            mangled_names.push(mn);
        }
        w!(self, r#"}}"#);
        match tr {
            Some(tr) => {
                let (tr, assocs) = tr.clone().take_assocs();
                w!(self, r#"impl {tr} for {owner} {{"#);
                for (name, ty) in assocs {
                    w!(self, r#"type {name} = {ty};"#);
                }
            }
            None => w!(self, r#"impl {owner} {{"#),
        }
        for (mn, method) in mangled_names.iter().zip(methods) {
            if tr.is_none() {
                w!(self, "pub ");
            }
            w!(
                self,
                r#"{}fn {}("#,
                if method.is_safe { "" } else { "unsafe " },
                method.name
            );
            match method.receiver {
                ZngurMethodReceiver::Static => (),
                ZngurMethodReceiver::Ref(Mutability::Mut) => w!(self, "&mut self, "),
                ZngurMethodReceiver::Ref(Mutability::Not) => w!(self, "&self, "),
                ZngurMethodReceiver::Move => w!(self, "self, "),
            }
            let input_offset = if method.receiver == ZngurMethodReceiver::Static {
                0
            } else {
                1
            };
            for (ty, n) in method.inputs.iter().zip(input_offset..) {
                w!(self, "i{n}: {ty}, ");
            }
            wln!(self, ") -> {} {{ unsafe {{", method.output);
            if method.receiver != ZngurMethodReceiver::Static {
                wln!(self, "let i0 = self;");
            }
            self.call_cpp_function(&format!("{mn}("), method.inputs.len() + input_offset);
            wln!(self, "}} }}");
        }
        w!(self, r#"}}"#);
        mangled_names
    }

    pub fn add_extern_cpp_function(
        &mut self,
        rust_name: &str,
        inputs: &[RustType],
        output: &RustType,
        is_safe: bool,
    ) -> String {
        let mangled_name = self.mangle_name(rust_name);
        w!(
            self,
            r#"
unsafe extern "C" {{ fn {mangled_name}("#
        );
        for (n, _) in inputs.iter().enumerate() {
            w!(self, "i{n}: *mut u8, ");
        }
        wln!(self, r#"o: *mut u8); }}"#);
        w!(
            self,
            r#"
#[allow(non_snake_case)]
pub {}fn {rust_name}("#,
            if is_safe { "" } else { "unsafe " }
        );
        for (n, ty) in inputs.iter().enumerate() {
            w!(self, "i{n}: {ty}, ");
        }
        wln!(self, ") -> {output} {{ unsafe {{");
        self.call_cpp_function(&format!("{mangled_name}("), inputs.len());
        wln!(self, "}} }}");
        mangled_name
    }

    pub fn add_cpp_heap_allocated_bridge(&mut self, ty: &RustType) -> String {
        let wrapper_ref = match ty {
            RustType::Cpp(_) => ty.to_string(),
            _ => ty.to_string().split("::").last().unwrap().to_string(),
        };
        let mangled_name = self.mangle_name(&format!("{ty}_cpp_heap_allocated"));
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {mangled_name}(d: *mut u8) -> *mut {wrapper_ref} {{
    d as *mut {wrapper_ref}
}}"#
        );
        mangled_name
    }

    pub fn add_function(
        &mut self,
        cxx_name: &str,
        rust_name: &str,
        inputs: &[RustType],
        output: &RustType,
        use_path: Option<String>,
        deref: Option<Mutability>,
        namespace: &str,
        crate_name: &str,
    ) -> CppFnSig {
        let mut mangled_name =
            self.mangle_name(&format!("{cxx_name}={rust_name}")) + "_" + &hash_of_sig(&inputs);
        if deref.is_some() {
            mangled_name += "_deref";
        }
        w!(
            self,
            r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
#[allow(unused_parens)]
pub extern "C" fn {mangled_name}("#
        );
        for n in 0..inputs.len() {
            w!(self, "i{n}: *mut u8, ");
        }
        let (modified_output, is_impl_trait) = if let RustType::Impl(tr, bounds) = output {
            (
                RustType::Boxed(Box::new(RustType::Dyn(tr.clone(), bounds.clone()))),
                true,
            )
        } else {
            (output.clone(), false)
        };
        wln!(self, "o: *mut u8) {{ unsafe {{");
        self.wrap_in_catch_unwind(|this| {
            if let Some(use_path) = use_path {
                wln!(this, "    use {use_path};");
            }

            w!(
                this,
                "    ::std::ptr::write(o as *mut {modified_output}, {impl_trait} {rust_name}(",
                impl_trait = if is_impl_trait { "Box::new( " } else { "" },
            );
            match deref {
                Some(Mutability::Mut) => w!(this, "::std::ops::DerefMut::deref_mut"),
                Some(Mutability::Not) => w!(this, "::std::ops::Deref::deref"),
                None => {}
            }
            for (n, ty) in inputs.iter().enumerate() {
                w!(this, "(::std::ptr::read(i{n} as *mut {ty})), ");
            }
            if is_impl_trait {
                wln!(this, ")));");
            } else {
                wln!(this, "));");
            }
        });
        wln!(self, " }} }}");
        CppFnSig {
            rust_link_name: mangled_name,
            inputs: inputs
                .iter()
                .map(|ty| ty.into_cpp(namespace, crate_name))
                .collect(),
            output: modified_output.into_cpp(namespace, crate_name),
        }
    }

    pub(crate) fn add_wellknown_trait(
        &mut self,
        ty: &RustType,
        wellknown_trait: ZngurWellknownTrait,
        is_unsized: bool,
    ) -> ZngurWellknownTraitData {
        match wellknown_trait {
            ZngurWellknownTrait::Unsized => ZngurWellknownTraitData::Unsized,
            ZngurWellknownTrait::Copy => ZngurWellknownTraitData::Copy,
            ZngurWellknownTrait::Drop => {
                let drop_in_place = self.mangle_name(&format!("{ty}=drop_in_place"));
                wln!(
                    self,
                    r#"
#[allow(non_snake_case)]
#[allow(dropping_copy_types)]
#[allow(dropping_references)]
#[allow(undropped_manually_drops)]
#[unsafe(no_mangle)]
pub extern "C" fn {drop_in_place}(v: *mut u8) {{ unsafe {{
    ::std::ptr::drop_in_place(v as *mut {ty});
}} }}"#
                );
                ZngurWellknownTraitData::Drop { drop_in_place }
            }
            ZngurWellknownTrait::Debug => {
                let pretty_print = self.mangle_name(&format!("{ty}=debug_pretty"));
                let debug_print = self.mangle_name(&format!("{ty}=debug_print"));
                let dbg_ty = if !is_unsized {
                    format!("{ty}")
                } else {
                    format!("&{ty}")
                };
                wln!(
                    self,
                    r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {pretty_print}(v: *mut u8) {{
    eprintln!("{{:#?}}", unsafe {{ &*(v as *mut {dbg_ty}) }});
}}"#
                );
                wln!(
                    self,
                    r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {debug_print}(v: *mut u8) {{
    eprintln!("{{:?}}", unsafe {{ &*(v as *mut {dbg_ty}) }});
}}"#
                );
                ZngurWellknownTraitData::Debug {
                    pretty_print,
                    debug_print,
                }
            }
        }
    }

    fn wrap_in_catch_unwind(&mut self, f: impl FnOnce(&mut RustFile)) {
        if !self.panic_to_exception {
            f(self);
        } else {
            wln!(
                self,
                r#"unsafe extern "C" {{
                fn __zngur_mark_panicked();   
            }}
            let e = ::std::panic::catch_unwind(|| {{"#
            );
            f(self);
            wln!(self, "}});");
            wln!(self, "if let Err(_) = e {{ __zngur_mark_panicked(); }}");
        }
    }

    /// Emits the Rust half of the coroutine support: shared waker shims plus
    /// per-output-type future shims. See [`CoroSupport`].
    pub fn add_coro_support(&mut self, coro: &CoroSupport) {
        let clone_waker_fn = &coro.clone_waker_fn;
        let waker_wake_fn = &coro.waker_wake_fn;
        let waker_drop_fn = &coro.waker_drop_fn;
        // The C++ side stores wakers in a fixed-size opaque slot
        // (`ZngurWakerSlot`), so verify at compile time that a `Waker` fits.
        wln!(
            self,
            r#"
const _: () = {{
    assert!(
        ::std::mem::size_of::<::std::task::Waker>() <= 64,
        "zngur: std::task::Waker does not fit in the C++ coroutine waker slot"
    );
    assert!(
        ::std::mem::align_of::<::std::task::Waker>() <= 16,
        "zngur: std::task::Waker alignment exceeds the C++ coroutine waker slot alignment"
    );
}};

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {clone_waker_fn}(src: *mut u8, dst: *mut u8) {{
    unsafe {{
        let src = &*(src as *const ::std::task::Waker);
        let cloned = src.clone();
        ::std::ptr::write(dst as *mut ::std::task::Waker, cloned);
    }}
}}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {waker_wake_fn}(w: *mut u8) {{
    unsafe {{
        let w = ::std::ptr::read(w as *mut ::std::task::Waker);
        w.wake();
    }}
}}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {waker_drop_fn}(w: *mut u8) {{
    unsafe {{
        ::std::ptr::drop_in_place(w as *mut ::std::task::Waker);
    }}
}}"#
        );
        for shim in &coro.shims {
            let ty = &shim.rust_output;
            let is_send_sync = shim.is_send_sync;
            let poll_future_fn = &shim.poll_future_fn;
            let make_coro_future_fn = &shim.make_coro_future_fn;
            let coro_poll_fn = &shim.coro_poll_fn;
            let coro_destroy_fn = &shim.coro_destroy_fn;
            let send_sync_bound = if is_send_sync { " + Send + Sync" } else { "" };
            wln!(
                self,
                r#"
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {poll_future_fn}(fut: *mut u8, waker: *mut u8, out: *mut u8) -> u8 {{
    unsafe {{
        let fut = &mut *(fut as *mut ::std::boxed::Box<dyn ::std::future::Future<Output = {ty}>{send_sync_bound}>);
        let waker = &*(waker as *const ::std::task::Waker);
        let mut cx = ::std::task::Context::from_waker(waker);
        match ::std::pin::Pin::new_unchecked(fut.as_mut()).poll(&mut cx) {{
            ::std::task::Poll::Ready(v) => {{
                ::std::ptr::write(out as *mut {ty}, v);
                1
            }}
            ::std::task::Poll::Pending => 0,
        }}
    }}
}}

unsafe extern "C" {{
    fn {coro_poll_fn}(handle: *mut u8, waker: *mut u8, out: *mut u8) -> u8;
    fn {coro_destroy_fn}(handle: *mut u8);
}}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn {make_coro_future_fn}(handle: *mut u8, out: *mut u8) {{
    struct CoroState {{
        handle: *mut u8,
    }}
    impl ::std::future::Future for CoroState {{
        type Output = {ty};
        fn poll(
            self: ::std::pin::Pin<&mut Self>,
            cx: &mut ::std::task::Context<'_>,
        ) -> ::std::task::Poll<{ty}> {{
            let mut out_val = ::std::mem::MaybeUninit::<{ty}>::uninit();
            let waker_ptr = cx.waker() as *const ::std::task::Waker as *mut u8;
            let is_ready =
                unsafe {{ {coro_poll_fn}(self.handle, waker_ptr, out_val.as_mut_ptr() as *mut u8) }};
            if is_ready != 0 {{
                ::std::task::Poll::Ready(unsafe {{ out_val.assume_init() }})
            }} else {{
                ::std::task::Poll::Pending
            }}
        }}
    }}
    impl Drop for CoroState {{
        fn drop(&mut self) {{
            unsafe {{ {coro_destroy_fn}(self.handle) }}
        }}
    }}"#
            );
            if is_send_sync {
                wln!(
                    self,
                    "    unsafe impl Send for CoroState {{}} unsafe impl Sync for CoroState {{}}"
                );
            }
            wln!(
                self,
                r#"    let state = CoroState {{ handle }};
    let boxed: ::std::boxed::Box<dyn ::std::future::Future<Output = {ty}>{send_sync_bound}> =
        ::std::boxed::Box::new(state);
    unsafe {{
        ::std::ptr::write(
            out as *mut ::std::boxed::Box<dyn ::std::future::Future<Output = {ty}>{send_sync_bound}>,
            boxed,
        )
    }}
}}"#
            );
        }
    }

    pub(crate) fn add_layout_policy_shim(
        &mut self,
        ty: &RustType,
        layout: LayoutPolicy,
    ) -> CppLayoutPolicy {
        match layout {
            LayoutPolicy::StackAllocated { size, align } => {
                CppLayoutPolicy::StackAllocated { size, align }
            }
            LayoutPolicy::Conservative { size, align } => {
                CppLayoutPolicy::StackAllocated { size, align }
            }
            LayoutPolicy::HeapAllocated => {
                let size_fn = self.mangle_name(&format!("{ty}_size_fn"));
                let alloc_fn = self.mangle_name(&format!("{ty}_alloc_fn"));
                let free_fn = self.mangle_name(&format!("{ty}_free_fn"));
                wln!(
                    self,
                    r#"
                #[allow(non_snake_case)]
                #[unsafe(no_mangle)]
                pub fn {size_fn}() -> usize {{
                    ::std::mem::size_of::<{ty}>()
                }}
        
                #[allow(non_snake_case)]
                #[unsafe(no_mangle)]
                pub fn {alloc_fn}() -> *mut u8 {{
                    unsafe {{ ::std::alloc::alloc(::std::alloc::Layout::new::<{ty}>()) }}
                }}

                #[allow(non_snake_case)]
                #[unsafe(no_mangle)]
                pub fn {free_fn}(p: *mut u8) {{
                    unsafe {{ ::std::alloc::dealloc(p, ::std::alloc::Layout::new::<{ty}>()) }}
                }}
                "#
                );
                CppLayoutPolicy::HeapAllocated {
                    size_fn,
                    alloc_fn,
                    free_fn,
                }
            }
            LayoutPolicy::OnlyByRef => CppLayoutPolicy::OnlyByRef,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_into_cpp() {
        let cpp = RustType::Cpp(vec!["a".to_owned(), "Name".to_owned()]);
        let cpp_type = cpp.into_cpp("rust", "my_crate");
        assert_eq!(cpp_type.to_string(), "::rust::a::Name");
    }
}
