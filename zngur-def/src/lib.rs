use std::fmt::Display;

use indexmap::IndexMap;
use itertools::Itertools;

#[cfg(feature = "printing")]
pub mod printing;

mod merge;
pub use merge::{ConflictSource, Merge, MergeFailure, MergeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Mut,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZngurMethodReceiver {
    Static,
    Ref(Mutability),
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurMethod {
    pub name: String,
    pub generics: Vec<RustType>,
    pub receiver: ZngurMethodReceiver,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurFn {
    pub path: RustPathAndGenerics,
    pub inputs: Vec<RustType>,
    pub output: RustType,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppFn {
    pub name: String,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppImpl {
    pub tr: Option<RustTrait>,
    pub ty: RustType,
    pub methods: Vec<ZngurMethod>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurConstructor {
    pub inputs: Vec<(String, RustType)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurVariant {
    pub name: String,
    pub fields: Vec<ZngurField>,
    pub exhaustive: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurField {
    pub name: String,
    pub ty: RustType,
    pub offset: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurFieldData {
    pub name: String,
    pub ty: RustType,
    pub offset: ZngurFieldDataOffset,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZngurFieldDataOffset {
    Offset(usize),
    Auto(String),
    AutoDynamic(String),
}

impl ZngurFieldDataOffset {
    pub fn is_offset(&self) -> bool {
        matches!(self, ZngurFieldDataOffset::Offset(_))
    }

    pub fn as_offset(&self) -> Option<usize> {
        match self {
            ZngurFieldDataOffset::Offset(o) => Some(*o),
            _ => None,
        }
    }

    pub fn as_auto(&self) -> Option<&str> {
        match self {
            ZngurFieldDataOffset::Auto(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZngurWellknownTrait {
    Debug,
    Drop,
    Unsized,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZngurWellknownTraitData {
    Debug {
        pretty_print: String,
        debug_print: String,
    },
    Drop {
        drop_in_place: String,
    },
    Unsized,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPolicy {
    StackAllocated { size: usize, align: usize },
    Conservative { size: usize, align: usize },
    HeapAllocated,
    OnlyByRef,
}

impl LayoutPolicy {
    pub const ZERO_SIZED_TYPE: Self = LayoutPolicy::StackAllocated { size: 0, align: 1 };
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurMethodDetails {
    pub data: ZngurMethod,
    pub use_path: Option<Vec<String>>,
    pub deref: Option<(RustType, Mutability)>,
    pub cpp_name: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppHeapAllocated(pub String);

#[derive(Debug)]
pub struct CppHeapAllocatedData {
    pub bridge_fn: String,
    pub cpp_type: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppRef(pub String);

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppStackOwned {
    pub cpp_type: String,
    pub size: usize,
    pub align: usize,
}

impl Display for CppRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, PartialEq)]
pub struct ZngurType {
    pub ty: RustType,
    pub layout: Option<LayoutPolicy>,
    pub wellknown_traits: Vec<ZngurWellknownTrait>,
    pub exhaustive: bool,
    pub methods: Vec<ZngurMethodDetails>,
    pub constructor: Option<ZngurConstructor>,
    pub variants: Vec<ZngurVariant>,
    pub fields: Vec<ZngurField>,
    pub cpp_heap_allocated: Option<CppHeapAllocated>,
    pub cpp_ref: Option<CppRef>,
    pub cpp_stack_owned: Option<CppStackOwned>,
}

#[derive(Debug, PartialEq)]
pub struct ZngurTrait {
    pub tr: RustTrait,
    pub methods: Vec<ZngurMethod>,
}

#[derive(Debug, Default)]
pub struct AdditionalIncludes(pub String);

#[derive(Debug, Default)]
pub struct ConvertPanicToException(pub bool);

#[derive(Clone, Debug, Default)]
pub struct Import(pub std::path::PathBuf);

#[derive(Debug, Clone)]
pub struct ModuleImport {
    pub path: std::path::PathBuf,
}

#[derive(Debug, Default)]
pub struct ZngurSpec {
    pub imported_modules: Vec<ModuleImport>,
    pub types: Vec<ZngurType>,
    pub traits: IndexMap<RustTrait, ZngurTrait>,
    pub funcs: Vec<ZngurFn>,
    pub extern_cpp_funcs: Vec<ZngurExternCppFn>,
    pub extern_cpp_impls: Vec<ZngurExternCppImpl>,
    pub additional_includes: AdditionalIncludes,
    pub convert_panic_to_exception: ConvertPanicToException,
    pub cpp_include_header_name: String,
    pub mangling_base: String,
    pub cpp_namespace: Option<String>,
    pub rust_cfg: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RustTrait {
    Normal(RustPathAndGenerics),
    Fn {
        name: String,
        inputs: Vec<RustType>,
        output: Box<RustType>,
    },
}

impl RustTrait {
    pub fn take_assocs(mut self) -> (Self, Vec<(String, RustType)>) {
        let assocs = match &mut self {
            RustTrait::Normal(p) => std::mem::take(&mut p.named_generics),
            RustTrait::Fn { .. } => vec![],
        };
        (self, assocs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrimitiveRustType {
    Uint(u32),
    Int(u32),
    Float(u32),
    Usize,
    Bool,
    Char,
    Str,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeVar(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RustPathAndGenerics {
    pub path: Vec<String>,
    pub generics: Vec<RustType>,
    pub named_generics: Vec<(String, RustType)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RustType {
    Primitive(PrimitiveRustType),
    Ref(Mutability, Box<RustType>),
    Raw(Mutability, Box<RustType>),
    Boxed(Box<RustType>),
    Slice(Box<RustType>),
    Dyn(RustTrait, Vec<String>),
    Impl(RustTrait, Vec<String>),
    Tuple(Vec<RustType>),
    Adt(RustPathAndGenerics),
    TypeVar(TypeVar),
}

impl RustType {
    pub const UNIT: Self = RustType::Tuple(Vec::new());
}

impl Display for RustPathAndGenerics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RustPathAndGenerics {
            path,
            generics,
            named_generics,
        } = self;
        for p in path {
            if p != "crate" {
                write!(f, "::")?;
            }
            write!(f, "{p}")?;
        }
        if !generics.is_empty() || !named_generics.is_empty() {
            write!(
                f,
                "::<{}>",
                generics
                    .iter()
                    .map(|x| format!("{x}"))
                    .chain(named_generics.iter().map(|x| format!("{} = {}", x.0, x.1)))
                    .join(", ")
            )?;
        }
        Ok(())
    }
}

impl Display for RustTrait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustTrait::Normal(tr) => write!(f, "{tr}"),
            RustTrait::Fn {
                name,
                inputs,
                output,
            } => {
                write!(f, "{name}({})", inputs.iter().join(", "))?;
                if **output != RustType::UNIT {
                    write!(f, " -> {output}")?;
                }
                Ok(())
            }
        }
    }
}

impl Display for PrimitiveRustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimitiveRustType::Uint(s) => write!(f, "u{s}"),
            PrimitiveRustType::Int(s) => write!(f, "i{s}"),
            PrimitiveRustType::Float(s) => write!(f, "f{s}"),
            PrimitiveRustType::Usize => write!(f, "usize"),
            PrimitiveRustType::Bool => write!(f, "bool"),
            PrimitiveRustType::Char => write!(f, "char"),
            PrimitiveRustType::Str => write!(f, "str"),
        }
    }
}

impl Display for RustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustType::Primitive(s) => write!(f, "{s}"),
            RustType::Ref(Mutability::Not, ty) => write!(f, "&{ty}"),
            RustType::Ref(Mutability::Mut, ty) => write!(f, "&mut {ty}"),
            RustType::Raw(Mutability::Not, ty) => write!(f, "*const {ty}"),
            RustType::Raw(Mutability::Mut, ty) => write!(f, "*mut {ty}"),
            RustType::Boxed(ty) => write!(f, "Box<{ty}>"),
            RustType::Tuple(v) => write!(f, "({})", v.iter().join(", ")),
            RustType::Adt(pg) => write!(f, "{pg}"),
            RustType::Dyn(tr, marker_bounds) => {
                write!(f, "dyn {tr}")?;
                for mb in marker_bounds {
                    write!(f, "+ {mb}")?;
                }
                Ok(())
            }
            RustType::Impl(tr, marker_bounds) => {
                write!(f, "impl {tr}")?;
                for mb in marker_bounds {
                    write!(f, "+ {mb}")?;
                }
                Ok(())
            }
            RustType::Slice(s) => write!(f, "[{s}]"),
            RustType::TypeVar(TypeVar(v)) => write!(f, "{v}"),
        }
    }
}
