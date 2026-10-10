use std::{
    fmt::{Display, Write},
    iter,
};

use indexmap::IndexMap;
use itertools::Itertools;
use zngur_def::{
    CppHeapAllocatedData, CppRef, CppStackOwned, RustTrait, ZngurFieldData, ZngurMethodReceiver,
};

use crate::{
    ZngurWellknownTraitData,
    template::{CppHeaderTemplate, CppSourceTemplate},
};
use askama::Template;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppPath(pub Vec<String>);

impl CppPath {
    fn namespace(&self) -> &[String] {
        self.0.split_last().unwrap().1
    }

    pub(crate) fn open_namespace(&self) -> String {
        self.namespace()
            .iter()
            .enumerate()
            .map(|(i, x)| format!("{:indent$}namespace {} {{", "", x, indent = i * 4))
            .join("\n")
    }

    pub(crate) fn close_namespace(&self) -> String {
        self.namespace()
            .iter()
            .enumerate()
            .map(|(i, x)| format!("{:indent$}}} // namespace {}", "", x, indent = i * 4))
            .join("\n")
    }

    pub(crate) fn name(&self) -> &str {
        match self.0.split_last().unwrap().0.as_str() {
            // Unit is a valid type alias for Tuple<> except when defining the constructors
            "Unit" => "Tuple",
            s => s,
        }
    }

    fn need_header(&self) -> bool {
        if self.0.len() == 1 && self.0[0].ends_with("_t") {
            return false;
        }
        // Top level namespace and type...
        if self.0.len() == 2 {
            let ty = &self.0[1];
            return ty != "Unit" && ty != "Ref" && ty != "RefMut";
        }
        true
    }

    pub(crate) fn from_rust_path(path: &[String], ns: &str, crate_name: &str) -> CppPath {
        CppPath(
            iter::once(ns)
                .chain(
                    path.iter()
                        .map(|x| if x == "crate" { crate_name } else { x.as_str() }),
                )
                .map(cpp_handle_keyword)
                .map(|x| x.to_owned())
                .collect(),
        )
    }
}

impl<const N: usize> From<[&str; N]> for CppPath {
    fn from(value: [&str; N]) -> Self {
        CppPath(value.iter().map(|x| x.to_string()).collect())
    }
}

impl From<&str> for CppPath {
    fn from(value: &str) -> Self {
        let value = value.trim();
        CppPath(value.split("::").map(|x| x.to_owned()).collect())
    }
}

impl Display for CppPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "::{}", self.0.iter().join("::"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppType {
    pub path: CppPath,
    pub generic_args: Vec<CppType>,
    // TODO: is there a better way to do it?
    pub tail: Option<String>,
}

impl CppType {
    pub fn with_tail(self, tail: String) -> Self {
        Self {
            tail: Some(tail),
            ..self
        }
    }

    pub(crate) fn specialization_decl(&self) -> String {
        if self.tail.is_some() {
            panic!("specialization decl makes no sense for nested classes");
        }

        let name = self.path.name();
        // Tuple<> is a little special because it is a template even though self.generic_args is empty
        if self.generic_args.is_empty() && name != "Tuple" {
            format!("struct {}", name)
        } else {
            format!(
                "template<> struct {}< {} >",
                name,
                self.generic_args.iter().join(", ")
            )
        }
    }

    pub(crate) fn specialized_name(&self) -> String {
        // We don't get `std::fmt::from_fn()` on our MSRV.
        struct Helper<'a>(&'a CppType);

        impl Display for Helper<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0.path.name())?;
                if !self.0.generic_args.is_empty() {
                    write!(f, "< {} >", self.0.generic_args.iter().join(", "))?;
                }
                if let Some(tail) = &self.0.tail {
                    write!(f, " :: {tail}")?;
                }
                Ok(())
            }
        }

        Helper(self).to_string()
    }

    fn header_helper(&self, state: &mut impl Write) -> std::fmt::Result {
        // Note: probably need to keep this out of the template because it's recursive.
        for x in &self.generic_args {
            x.header_helper(state)?;
        }

        if !self.path.need_header() {
            return Ok(());
        }

        for p in self.path.namespace() {
            writeln!(state, "namespace {} {{", p)?;
        }

        if !self.generic_args.is_empty() {
            writeln!(state, "template<typename ...T>")?;
        }

        writeln!(state, "struct {};", self.path.name())?;

        for _ in self.path.namespace() {
            writeln!(state, "}}")?;
        }

        Ok(())
    }

    pub(crate) fn header(&self) -> String {
        let mut state = String::new();

        self.header_helper(&mut state).unwrap();

        state
    }
}

impl Display for CppType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)?;
        if !self.generic_args.is_empty() {
            write!(f, "< {} >", self.generic_args.iter().join(", "))?;
        }
        Ok(())
    }
}

fn split_string(input: &str) -> impl Iterator<Item = String> {
    let mut parts = Vec::new();
    let mut current_part = String::new();
    let mut parentheses_count = 0;

    for c in input.chars() {
        match c {
            ',' if parentheses_count == 0 => {
                parts.push(current_part.clone());
                current_part.clear();
            }
            '<' => {
                parentheses_count += 1;
                current_part.push(c);
            }
            '>' => {
                parentheses_count -= 1;
                current_part.push(c);
            }
            _ => {
                current_part.push(c);
            }
        }
    }

    if !current_part.is_empty() {
        parts.push(current_part);
    }

    parts.into_iter()
}

impl From<&str> for CppType {
    fn from(value: &str) -> Self {
        let value = value.trim();
        match value.split_once('<') {
            None => CppType {
                path: CppPath::from(value),
                generic_args: vec![],
                tail: None,
            },
            Some((path, generics)) => {
                let generics = generics.strip_suffix('>').unwrap();
                CppType {
                    path: CppPath::from(path),
                    generic_args: split_string(generics).map(|x| CppType::from(&*x)).collect(),
                    tail: None,
                }
            }
        }
    }
}

// pub(crate) just for migration
pub(crate) struct State {
    pub(crate) text: String,
    pub(crate) panic_to_exception: bool,
}

impl State {
    fn remove_no_except_in_panic(&mut self) {
        if self.panic_to_exception {
            self.text = self.text.replace(" noexcept ", " ");
        }
    }
}

impl Write for State {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.text += s;
        Ok(())
    }
}

#[derive(Debug)]
pub struct CppTraitMethod {
    pub name: String,
    pub rust_link_name: String,
    pub inputs: Vec<CppType>,
    pub output: CppType,
}

impl CppTraitMethod {
    pub fn render_inputs(&self, namespace: &str) -> String {
        self.inputs
            .iter()
            .enumerate()
            .map(|(n, ty)| {
                format!(
                    "::{}::__zngur_internal_move_from_rust< {ty} >(i{n})",
                    namespace
                )
            })
            .join(", ")
    }
}

#[derive(Debug)]
pub struct CppFnSig {
    pub rust_link_name: String,
    pub inputs: Vec<CppType>,
    pub output: CppType,
}

impl CppFnSig {
    pub fn render_inputs(&self, namespace: &str) -> String {
        self.inputs
            .iter()
            .enumerate()
            .map(|(n, ty)| {
                format!(
                    "::{}::__zngur_internal_move_from_rust< {ty} >(i{n})",
                    namespace
                )
            })
            .join(", ")
    }
}

pub struct CppFnDefinition {
    pub name: CppPath,
    pub sig: CppFnSig,
}

pub struct CppExportedFnDefinition {
    pub name: String,
    pub sig: CppFnSig,
}

pub struct CppExportedImplDefinition {
    pub tr: Option<CppType>,
    pub ty: CppType,
    pub methods: Vec<(String, CppFnSig)>,
}

impl CppExportedImplDefinition {
    pub fn render_tr(&self, namespace: &str) -> String {
        match &self.tr {
            Some(x) => format!("{x}"),
            None => format!("::{}::Inherent", namespace),
        }
    }
}

#[derive(Debug)]
pub struct CppMethod {
    pub name: String,
    pub kind: ZngurMethodReceiver,
    pub sig: CppFnSig,
}

impl CppMethod {
    pub fn is_valid_field_method(&self, field_kind: &str) -> bool {
        if let zngur_def::ZngurMethodReceiver::Ref(m) = &self.kind {
            !(*m == zngur_def::Mutability::Mut && field_kind == "FieldRef")
        } else {
            false
        }
    }

    pub fn is_ref_not_mut(&self) -> bool {
        matches!(
            self.kind,
            zngur_def::ZngurMethodReceiver::Ref(zngur_def::Mutability::Not)
        )
    }

    pub fn fn_name(&self, type_name: &str) -> String {
        format!("{}::{}", type_name, self.name)
    }

    pub fn render_sig_inputs_skip_one(&self) -> String {
        use itertools::Itertools;
        self.sig
            .inputs
            .iter()
            .skip(1)
            .enumerate()
            .map(|(n, ty)| format!("{ty} i{n}"))
            .join(", ")
    }
}

#[derive(Debug)]
pub struct CppVariant {
    pub name: String,
    pub constructor: Option<CppFnSig>,
    pub match_check: String,
    pub fields: Vec<ZngurFieldData>,
}

#[derive(Debug)]
pub enum CppTraitDefinition {
    Fn {
        sig: CppFnSig,
    },
    Normal {
        as_ty: CppType,
        methods: Vec<CppTraitMethod>,
        link_name: String,
        link_name_ref: String,
    },
}

impl CppTraitDefinition {
    pub fn is_normal(&self) -> bool {
        matches!(self, CppTraitDefinition::Normal { .. })
    }

    pub fn as_normal(&self) -> Option<(&CppType, &[CppTraitMethod], &str, &str)> {
        if let CppTraitDefinition::Normal {
            as_ty,
            methods,
            link_name,
            link_name_ref,
        } = self
        {
            Some((as_ty, methods, link_name, link_name_ref))
        } else {
            None
        }
    }

    pub fn as_fn(&self) -> Option<&CppFnSig> {
        if let CppTraitDefinition::Fn { sig } = self {
            Some(sig)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CppLayoutPolicy {
    StackAllocated {
        size: usize,
        align: usize,
    },
    HeapAllocated {
        size_fn: String,
        alloc_fn: String,
        free_fn: String,
    },
    OnlyByRef,
}

impl CppLayoutPolicy {
    pub fn is_only_by_ref(&self) -> bool {
        matches!(self, CppLayoutPolicy::OnlyByRef)
    }

    pub fn is_stack_allocated(&self) -> bool {
        matches!(self, CppLayoutPolicy::StackAllocated { .. })
    }

    pub fn stack_size(&self) -> usize {
        if let CppLayoutPolicy::StackAllocated { size, .. } = self {
            *size
        } else {
            0
        }
    }

    pub fn stack_align(&self) -> usize {
        if let CppLayoutPolicy::StackAllocated { align, .. } = self {
            *align
        } else {
            1
        }
    }

    pub fn alloc_heap(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { alloc_fn, .. } = self {
            format!("__zngur_data = {}();", alloc_fn)
        } else {
            "".to_owned()
        }
    }

    pub fn free_heap(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { free_fn, .. } = self {
            format!("{}(__zngur_data);", free_fn)
        } else {
            "".to_owned()
        }
    }

    pub fn copy_data(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { size_fn, .. } = self {
            format!(
                "memcpy(this->__zngur_data, other.__zngur_data, {}());",
                size_fn
            )
        } else {
            "this->__zngur_data = other.__zngur_data;".to_owned()
        }
    }

    pub fn size_fn(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { size_fn, .. } = self {
            size_fn.clone()
        } else {
            "".to_owned()
        }
    }

    pub fn alloc_fn(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { alloc_fn, .. } = self {
            alloc_fn.clone()
        } else {
            "".to_owned()
        }
    }

    pub fn free_fn(&self) -> String {
        if let CppLayoutPolicy::HeapAllocated { free_fn, .. } = self {
            free_fn.clone()
        } else {
            "".to_owned()
        }
    }
}

#[derive(Debug)]
pub struct CppTypeDefinition {
    pub ty: CppType,
    pub layout: CppLayoutPolicy,
    pub layout_size: usize,
    pub layout_align: usize,
    pub is_layout_conservative: bool,
    pub methods: Vec<CppMethod>,
    pub constructor: Option<CppFnSig>,
    pub variants: Vec<CppVariant>,
    pub discriminant: Option<String>,
    pub fields: Vec<ZngurFieldData>,
    pub from_trait: Option<RustTrait>,
    pub from_trait_ref: Option<RustTrait>,
    pub wellknown_traits: Vec<ZngurWellknownTraitData>,
    pub cpp_heap_allocated: Option<CppHeapAllocatedData>,
    pub cpp_ref: Option<CppRef>,
    pub cpp_stack_owned: Option<CppStackOwned>,
}

impl CppTypeDefinition {
    pub fn rust_short_name(&self) -> String {
        self.ty.to_string().split("::").last().unwrap().to_string()
    }

    pub fn cpp_short_name(&self) -> String {
        if let Some(c) = &self.cpp_stack_owned {
            c.0.split("::").last().unwrap().to_string()
        } else {
            "".to_string()
        }
    }

    pub fn has_unsized(&self) -> bool {
        self.wellknown_traits
            .iter()
            .any(|t| matches!(t, ZngurWellknownTraitData::Unsized))
    }

    pub fn has_copy(&self) -> bool {
        self.wellknown_traits
            .iter()
            .any(|t| matches!(t, ZngurWellknownTraitData::Copy))
    }

    pub fn drop_in_place(&self) -> String {
        self.wellknown_traits
            .iter()
            .find_map(|t| {
                if let ZngurWellknownTraitData::Drop { drop_in_place } = t {
                    Some(drop_in_place.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }

    pub fn render_make_box(&self, name: &str, namespace: &str, crate_name: &str) -> String {
        use crate::rust::IntoCpp;
        use itertools::Itertools;
        match &self.from_trait {
            Some(RustTrait::Fn { inputs, output, .. }) => {
                let out_ty = output.into_cpp(namespace, crate_name);
                let in_tys = inputs
                    .iter()
                    .map(|x| x.into_cpp(namespace, crate_name))
                    .join(", ");
                format!(
                    "static inline {} make_box(::std::function<{} ({})> f);",
                    name, out_ty, in_tys
                )
            }
            Some(RustTrait::Normal(_)) => {
                format!(
                    "template<typename T, typename... Args>\nstatic inline {} make_box(Args&&... args);",
                    name
                )
            }
            None => "".to_owned(),
        }
    }

    pub fn render_make_box_ref(&self, name: &str, namespace: &str, crate_name: &str) -> String {
        use crate::rust::IntoCpp;
        use itertools::Itertools;
        match &self.from_trait_ref {
            Some(RustTrait::Fn { inputs, output, .. }) => {
                let out_ty = output.into_cpp(namespace, crate_name);
                let in_tys = inputs
                    .iter()
                    .map(|x| x.into_cpp(namespace, crate_name))
                    .join(", ");
                format!(
                    "inline {}(::std::function<{} ({})> f);",
                    name, out_ty, in_tys
                )
            }
            Some(tr @ RustTrait::Normal(_)) => {
                format!(
                    "inline RefMut({}& arg);",
                    tr.into_cpp(namespace, crate_name)
                )
            }
            None => "".to_owned(),
        }
    }

    pub fn render_make_box_ref_only(
        &self,
        name: &str,
        namespace: &str,
        crate_name: &str,
    ) -> String {
        use crate::rust::IntoCpp;
        use itertools::Itertools;
        match &self.from_trait_ref {
            Some(RustTrait::Fn { inputs, output, .. }) => {
                let out_ty = output.into_cpp(namespace, crate_name);
                let in_tys = inputs
                    .iter()
                    .map(|x| x.into_cpp(namespace, crate_name))
                    .join(", ");
                format!(
                    "inline {}(::std::function<{} ({})> f);",
                    name, out_ty, in_tys
                )
            }
            Some(tr @ RustTrait::Normal(_)) => {
                format!("inline Ref({}& arg);", tr.into_cpp(namespace, crate_name))
            }
            None => "".to_owned(),
        }
    }
}

impl Default for CppTypeDefinition {
    fn default() -> Self {
        Self {
            ty: CppType::from("fill::me::you::forgot::it"),
            layout: CppLayoutPolicy::OnlyByRef,
            layout_size: 0,
            layout_align: 1,
            is_layout_conservative: false,
            methods: vec![],
            constructor: None,
            variants: vec![],
            discriminant: None,
            fields: vec![],
            wellknown_traits: vec![],
            from_trait: None,
            from_trait_ref: None,
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        }
    }
}

#[derive(Default)]
pub struct CppFile {
    pub header_file_name: String,
    pub type_defs: Vec<CppTypeDefinition>,
    pub trait_defs: IndexMap<RustTrait, CppTraitDefinition>,
    pub fn_defs: Vec<CppFnDefinition>,
    pub exported_fn_defs: Vec<CppExportedFnDefinition>,
    pub exported_impls: Vec<CppExportedImplDefinition>,
    pub additional_includes: String,
    pub panic_to_exception: bool,
    pub rust_cfg_defines: Vec<String>,
    pub zng_header_in_place: bool,
    pub coro_support: Option<crate::rust::CoroSupport>,
}

impl CppFile {
    fn emit_h_file(
        &self,
        state: &mut State,
        namespace: &str,
        crate_name: &str,
    ) -> std::fmt::Result {
        let template = CppHeaderTemplate {
            panic_to_exception: self.panic_to_exception,
            additional_includes: &self.additional_includes,
            fn_deps: &self.fn_defs,
            type_defs: &self.type_defs,
            trait_defs: &self.trait_defs,
            exported_impls: &self.exported_impls,
            exported_fn_defs: &self.exported_fn_defs,
            rust_cfg_defines: &self.rust_cfg_defines,
            zng_header_in_place: self.zng_header_in_place,
            coro_support: &self.coro_support,
            namespace,
            crate_name,
        };
        state.text += normalize_whitespace(template.render().unwrap().as_str()).as_str();
        Ok(())
    }

    fn emit_cpp_file(
        &self,
        state: &mut State,
        is_really_needed: &mut bool,
        namespace: &str,
    ) -> std::fmt::Result {
        let template = CppSourceTemplate {
            header_file_name: &self.header_file_name,
            type_defs: &self.type_defs,
            trait_defs: &self.trait_defs,
            exported_fn_defs: &self.exported_fn_defs,
            exported_impls: &self.exported_impls,
            cpp_namespace: namespace,
            coro_support: &self.coro_support,
        };
        state.text += normalize_whitespace(template.render().unwrap().as_str()).as_str();

        *is_really_needed = !self.trait_defs.is_empty()
            || !self.exported_fn_defs.is_empty()
            || !self.exported_impls.is_empty()
            || self.coro_support.is_some();

        Ok(())
    }

    pub fn render(self, namespace: &str, crate_name: &str) -> (String, Option<String>) {
        let mut h_file = State {
            text: "".to_owned(),
            panic_to_exception: self.panic_to_exception,
        };
        let mut cpp_file = State {
            text: "".to_owned(),
            panic_to_exception: self.panic_to_exception,
        };
        self.emit_h_file(&mut h_file, namespace, crate_name)
            .unwrap();
        let mut is_cpp_needed = false;
        self.emit_cpp_file(&mut cpp_file, &mut is_cpp_needed, namespace)
            .unwrap();
        h_file.remove_no_except_in_panic();

        (h_file.text, is_cpp_needed.then_some(cpp_file.text))
    }
}

pub fn cpp_handle_keyword(name: &str) -> &str {
    match name {
        "new" => "new_",
        "default" => "default_",
        x => x,
    }
}

pub fn cpp_handle_field_name(name: &str) -> String {
    if name.parse::<u32>().is_ok() {
        return format!("f{name}");
    }
    cpp_handle_keyword(name).to_owned()
}

trait CountCharMatchesExt {
    fn count_start_matches(&self, pred: impl Fn(char) -> bool) -> usize;
    fn count_matches(&self, pred: impl Fn(char) -> bool) -> usize;
}

impl CountCharMatchesExt for str {
    fn count_start_matches(&self, pred: impl Fn(char) -> bool) -> usize {
        let mut count = 0;
        for c in self.chars() {
            if pred(c) {
                count += 1;
            } else {
                break;
            }
        }
        count
    }
    fn count_matches(&self, pred: impl Fn(char) -> bool) -> usize {
        let mut count = 0;
        for c in self.chars() {
            if pred(c) {
                count += 1;
            }
        }
        count
    }
}

/// Normalize newlines and indents in cpp source code
///
/// A relatively naive algorithm intended only to correct excessive indentation and condense
/// unneeded newlines that result from the sailfish template rendering.
pub fn normalize_whitespace(cpp: &str) -> String {
    enum LineType {
        PreProc,
        NamespaceOpen,
        ExternOpen,
        BlockOpen,
        BlockClose,
        Statement,
        Unknown,
        Empty,
    }
    struct FormatState {
        indent: usize,
        last_indent: usize,
        last_line: LineType,
    }

    let mut state = FormatState {
        indent: 0,
        last_indent: 0,
        last_line: LineType::Empty,
    };
    let lines = cpp.lines();
    // builds vec of &str then `join`s them to reduce memory footprint and copies
    let mut out: Vec<&str> = Vec::new();

    fn count_open_pairs(line: &str, pairs: &[[char; 2]]) -> isize {
        let mut count: isize = 0;
        for [open, close] in pairs {
            count += line.count_matches(|c| c == *open) as isize;
            count -= line.count_matches(|c| c == *close) as isize;
        }
        count
    }

    fn trim_end_c_comments(line: &str) -> &str {
        let mut j: usize = 0;
        let mut prev = false;
        for (index, c) in line.char_indices() {
            let is_slash = c == '/';
            if is_slash && prev {
                // consecutive `//` means rest of line is a comment
                break;
            }
            prev = is_slash;
            j = index;
        }
        if j + 1 >= line.len() {
            j = line.len()
        }
        // SAFETY: `line.char_indices` returns valid indices
        unsafe { line.get_unchecked(0..j) }
    }

    fn line_type(line: &str) -> LineType {
        if line.trim().is_empty() {
            LineType::Empty
        } else if line.trim_start().starts_with('#') {
            LineType::PreProc
        } else if line.trim_start().starts_with("namespace") && line.ends_with('{') {
            LineType::NamespaceOpen
        } else if line.trim_start().starts_with("extern") && line.ends_with('{') {
            LineType::ExternOpen
        } else if line.ends_with('{') && count_open_pairs(line, &[['{', '}']]) > 0 {
            LineType::BlockOpen
        } else if (line.ends_with('}') || line.ends_with("};"))
            && count_open_pairs(line, &[['{', '}']]) < 0
        {
            LineType::BlockClose
        } else if line.ends_with(';') {
            LineType::Statement
        } else {
            LineType::Unknown
        }
    }

    let mut last_indent: usize = 0;
    let mut last_line: &str = "";
    let mut last_extra_indent: usize = 0;
    let mut indents: Vec<usize> = Vec::new();
    for line in lines {
        let trimmed = trim_end_c_comments(line).trim_end();
        let ty = line_type(trimmed);
        let indent = line.count_start_matches(char::is_whitespace);
        let mut emit_line = false;
        // don't indent this line
        let mut do_not_indent = false;
        // extra indent levels for this line
        let mut extra_indent = 0;
        // subtracted from indent
        let mut special_indent = 0;
        // when opening a indent level, don't indent this line
        let mut use_last_indent = false;
        match ty {
            LineType::PreProc => {
                do_not_indent = true;
                emit_line = true;
            }
            LineType::NamespaceOpen => {
                indents.clear();
                state.indent = 0;
                state.last_indent = 0;
                emit_line = true;
            }
            LineType::ExternOpen => {
                indents.clear();
                indents.push(4);
                state.indent = 4;
                state.last_indent = 0;
                emit_line = true;
                use_last_indent = true;
            }
            LineType::BlockOpen => {
                emit_line = true;
                indents.push(4);
                state.indent += 4;
                use_last_indent = true;
            }
            LineType::BlockClose => {
                emit_line = true;
                if let Some(n) = indents.pop() {
                    state.indent = state.indent.saturating_sub(n);
                }
            }
            LineType::Statement | LineType::Unknown => {
                let open_pairs =
                    count_open_pairs(last_line, &[['[', ']'], ['{', '}'], ['<', '>'], ['(', ')']]);
                if trimmed.ends_with("public:") || trimmed.ends_with("private:") {
                    special_indent = 2;
                } else if indent == last_indent {
                    extra_indent = last_extra_indent;
                } else if !matches!(
                    state.last_line,
                    LineType::BlockOpen | LineType::NamespaceOpen | LineType::ExternOpen
                ) && indent > last_indent
                    && open_pairs > 0
                {
                    extra_indent += 4;
                } else if extra_indent > 0
                    && !matches!(
                        state.last_line,
                        LineType::BlockOpen | LineType::NamespaceOpen | LineType::ExternOpen
                    )
                    && open_pairs < 0
                {
                    extra_indent = extra_indent.saturating_sub(4);
                }
                emit_line = true;
            }
            LineType::Empty => match &state.last_line {
                // preserve empty line if previous line held meaning
                LineType::Statement | LineType::Unknown | LineType::BlockClose => {
                    do_not_indent = false;
                    if !last_line.ends_with([',', '[', '<', '(']) {
                        emit_line = true
                    }
                }
                // eliminate all other empty lines
                _ => {}
            },
        }

        state.last_line = ty;
        last_line = trimmed;
        last_indent = indent;
        last_extra_indent = extra_indent;
        if emit_line {
            // emit indent
            // extra_indent is for *this* line and forward
            // special_indent is for *this* line only
            if !do_not_indent
                && (((use_last_indent && state.last_indent > 0)
                    || (!use_last_indent && state.indent > 0))
                    || extra_indent > 0
                    || special_indent > 0)
            {
                out.extend(
                    iter::once(" ").cycle().take(
                        (if use_last_indent {
                            state.last_indent
                        } else {
                            state.indent
                        } + extra_indent)
                            .saturating_sub(special_indent),
                    ),
                );
            }
            // emit line without it's source indent
            out.push(line.trim());
            out.push("\n");
        }
        state.last_indent = state.indent;
    }
    out.join("")
}
