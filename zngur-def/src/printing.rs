use std::io::Write;

use itertools::Itertools;

use crate::CppStackOwned;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IDLItem {
    Function,
    Type,
    TypeWellknownTrait,
    TypeLayout,
    TypeConstructor,
    TypeField,
    TypeVariant,
    TypeVariantField,
    TypeVariantConstructor,
    TypeMethod,
    Trait,
    ExternFn,
    ExternImpl,
}

/// prints the zng IDL to an output writer
/// uses it's own trait `WriteIDL` over `std::fmt::Display` to control:
///   - the output of item types
///   - formatting
///   -  and the path to core/std items like `Box`

#[derive(Debug)]
pub struct IDLPrinter<'a, W: Write> {
    enabled_items: Vec<IDLItem>,
    current_indent: usize,
    out: &'a mut W,
    no_std: bool,
}

pub const DEFAULT_IDL_ITEMS: [IDLItem; 13] = [
    IDLItem::Function,
    IDLItem::Type,
    IDLItem::TypeWellknownTrait,
    IDLItem::TypeLayout,
    IDLItem::TypeConstructor,
    IDLItem::TypeField,
    IDLItem::TypeVariant,
    IDLItem::TypeVariantField,
    IDLItem::TypeVariantConstructor,
    IDLItem::TypeMethod,
    IDLItem::Trait,
    IDLItem::ExternFn,
    IDLItem::ExternImpl,
];

impl<'a, W: Write> IDLPrinter<'a, W> {
    pub fn new(out: &'a mut W) -> Self {
        IDLPrinter {
            enabled_items: Vec::from(DEFAULT_IDL_ITEMS),
            current_indent: 0,
            out,
            no_std: false,
        }
    }

    pub fn enable_item(&mut self, item: IDLItem) -> bool {
        if self.enabled_items.contains(&item) {
            return false;
        }
        self.enabled_items.push(item);
        true
    }

    pub fn disable_item(&mut self, item: &IDLItem) -> bool {
        if let Some(pos) = self.enabled_items.iter().position(|i| i == item) {
            self.enabled_items.remove(pos);
            return true;
        }
        false
    }

    pub fn enable_items(&mut self, items: &[IDLItem]) -> bool {
        items.iter().copied().any(|item| self.enable_item(item))
    }

    pub fn disable_items(&mut self, items: &[IDLItem]) -> bool {
        items.into_iter().any(|item| self.disable_item(item))
    }

    pub fn enabled(&self, item: &IDLItem) -> bool {
        self.enabled_items.contains(item)
    }

    pub fn indent(&mut self) {
        self.current_indent += 1;
    }

    pub fn dedent(&mut self) {
        self.current_indent -= 1;
    }

    pub fn write_indent(&mut self) -> std::io::Result<()> {
        let indent = " ".repeat(self.current_indent * 4);
        write!(&mut self.out, "{indent}")
    }

    pub fn write_idl<I: WriteIDL + ?Sized>(&mut self, item: &I) -> std::io::Result<()> {
        item.write_idl(self)
    }

    pub fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        self.out.write_fmt(args)
    }

    pub fn std_path(&self) -> &'static str {
        if self.no_std { "::core" } else { "::std" }
    }
}

pub trait WriteIDL {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()>;
}

impl<T: WriteIDL + ?Sized> WriteIDL for Box<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        p.write_idl::<T>(&**self)
    }
}

impl<T: WriteIDL> WriteIDL for Vec<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        for (i, item) in self.iter().enumerate() {
            if i > 0 && i < self.len() {
                write!(p, ", ")?;
            }
            p.write_idl(item)?;
        }
        Ok(())
    }
}

impl<T: WriteIDL> WriteIDL for Option<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Some(t) => p.write_idl(t),
            Self::None => Ok(()),
        }
    }
}

impl WriteIDL for crate::RustType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Primitive(prim) => write!(p, "{prim}"),
            Self::Ref(m, ty) => {
                match m {
                    crate::Mutability::Not => {
                        write!(p, "&")?;
                    }
                    crate::Mutability::Mut => {
                        write!(p, "&mut ")?;
                    }
                }
                p.write_idl(ty)
            }
            Self::Raw(m, ty) => {
                match m {
                    crate::Mutability::Not => {
                        write!(p, "*const ")?;
                    }
                    crate::Mutability::Mut => {
                        write!(p, "*mut ")?;
                    }
                }
                p.write_idl(ty)
            }
            Self::Boxed(ty) => {
                write!(p, "{}::box::Box<", p.std_path())?;
                p.write_idl(ty)?;
                write!(p, ">")
            }
            Self::Slice(ty) => {
                write!(p, "[")?;
                p.write_idl(ty)?;
                write!(p, "]")
            }
            Self::Dyn(trt, marker_bounds) => {
                write!(p, "dyn ")?;
                p.write_idl(trt)?;
                for bound in marker_bounds {
                    write!(p, "+ {bound}")?;
                }
                Ok(())
            }
            Self::Impl(trt, marker_bounds) => {
                write!(p, "impl ")?;
                p.write_idl(trt)?;
                for bound in marker_bounds {
                    write!(p, "+ {bound}")?;
                }
                Ok(())
            }
            Self::Tuple(tys) => {
                write!(p, "(")?;
                p.write_idl(tys)?;
                write!(p, ")")
            }
            Self::Adt(pg) => p.write_idl(pg),
            Self::TypeVar(var) => p.write_idl(var),
        }
    }
}

impl WriteIDL for crate::RustTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Normal(path_and_generics) => p.write_idl(path_and_generics),
            Self::Fn {
                name,
                inputs,
                output,
            } => {
                write!(p, "{name}(")?;
                p.write_idl(inputs)?;
                write!(p, ")")?;
                if **output != crate::RustType::UNIT {
                    write!(p, " -> ")?;
                    p.write_idl(output)?;
                }
                Ok(())
            }
        }
    }
}

impl WriteIDL for crate::RustPathAndGenerics {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        let (first, rest): (_, &[String]) = match &self.path[..] {
            [first] => (first, &[]),
            [first, rest @ ..] => (first, rest),
            _ => {
                // probably an error, a path with no segments
                // return early to not print any generics because that would not be valid
                return Ok(());
            }
        };
        if first != "crate" {
            write!(p, "::")?;
        }
        write!(p, "{first}")?;
        for segment in rest {
            write!(p, "::{segment}")?;
        }

        if (self.generics.len() + self.named_generics.len()) > 0 {
            write!(p, "<")?;
            let mut generic_count = 0;
            for generic in &self.generics {
                if generic_count > 0 {
                    write!(p, ", ")?;
                }
                p.write_idl(generic)?;
                generic_count += 1;
            }
            for (name, generic) in &self.named_generics {
                if generic_count > 0 {
                    write!(p, ", ")?;
                }
                write!(p, "{name} = ")?;
                p.write_idl(generic)?;
                generic_count += 1;
            }
            write!(p, ">")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::TypeVar {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        write!(p, "{}", self.0)
    }
}

impl WriteIDL for crate::ZngurSpec {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if self.convert_panic_to_exception.0 {
            writeln!(p, "#convert_panic_to_exception")?;
        }
        if !self.types.is_empty() {
            if self.convert_panic_to_exception.0 {
                writeln!(p)?;
            }
            for (i, ty) in self.types.iter().enumerate() {
                if i > 0 && i < self.types.len() {
                    writeln!(p)?;
                }
                p.write_indent()?;
                p.write_idl(ty)?;
            }
        }
        if !self.funcs.is_empty() {
            if !self.types.is_empty() || self.convert_panic_to_exception.0 {
                writeln!(p)?;
            }
            for func in &self.funcs {
                p.write_idl(func)?;
            }
        }
        if !self.traits.is_empty() {
            if !self.funcs.is_empty() || !self.types.is_empty() || self.convert_panic_to_exception.0
            {
                writeln!(p)?;
            }
            for (i, trt) in self.traits.values().enumerate() {
                if i > 0 && i < self.traits.len() {
                    writeln!(p)?;
                }
                p.write_idl(trt)?;
            }
        }
        if !self.extern_cpp_funcs.is_empty() || !self.extern_cpp_impls.is_empty() {
            if !self.traits.is_empty()
                || !self.funcs.is_empty()
                || !self.types.is_empty()
                || self.convert_panic_to_exception.0
            {
                writeln!(p)?;
            }
            p.write_indent()?;
            writeln!(p, r#"extern "C++" {{"#)?;
            p.indent();
            if !self.extern_cpp_funcs.is_empty() {
                writeln!(p)?;
                for extern_fn in &self.extern_cpp_funcs {
                    p.write_indent()?;
                    p.write_idl(extern_fn)?;
                }
            }
            if !self.extern_cpp_impls.is_empty() {
                if !self.extern_cpp_funcs.is_empty() {
                    writeln!(p)?;
                }
                for extern_impl in &self.extern_cpp_impls {
                    p.write_indent()?;
                    p.write_idl(extern_impl)?;
                }
            }
            writeln!(p)?;
            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurFn {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Function) {
            write!(p, "fn ")?;
            p.write_idl(&self.path)?;
            write!(p, "(")?;
            p.write_idl(&self.inputs)?;
            write!(p, ")")?;
            if self.output != crate::RustType::UNIT {
                write!(p, " -> ")?;
                p.write_idl(&self.output)?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Type) {
            write!(p, "type ")?;
            p.write_idl(&self.ty)?;
            writeln!(p, " {{")?;
            p.indent();

            // don't duplicate layout info
            if self.cpp_ref.is_none() {
                p.write_indent()?;
                p.write_idl(&self.layout)?;
            }
            if self.cpp_heap_allocated.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_heap_allocated)?;
            }
            if self.cpp_ref.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_ref)?;
            }
            if self.cpp_stack_owned.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_stack_owned)?;
            }

            if p.enabled(&IDLItem::TypeWellknownTrait) {
                let drop_filtered: Vec<_> = self
                    .wellknown_traits
                    .iter()
                    .filter(|wk| !matches!(wk, crate::ZngurWellknownTrait::Drop))
                    .collect();
                if !drop_filtered.is_empty() {
                    p.write_indent()?;
                    write!(p, "wellknown_traits(")?;
                    p.write_idl(&self.wellknown_traits)?;
                    writeln!(p, ");")?;
                }
            }

            if self.constructor.is_some() {
                p.write_indent()?;
                p.write_idl(&self.constructor)?;
            }

            if p.enabled(&IDLItem::TypeVariant) {
                if !self.exhaustive {
                    p.write_indent()?;
                    writeln!(p, "non_exhaustive;")?;
                }
                for variant in &self.variants {
                    p.write_indent()?;
                    p.write_idl(variant)?;
                }
            }

            if !self.fields.is_empty() {
                writeln!(p)?;
                for field in &self.fields {
                    p.write_indent()?;
                    p.write_idl(field)?;
                }
            }

            if !self.methods.is_empty() {
                writeln!(p)?;
                for method in &self.methods {
                    p.write_indent()?;
                    p.write_idl(method)?;
                }
            }

            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Trait) {
            write!(p, "trait ")?;
            p.write_idl(&self.tr)?;
            write!(p, " {{")?;
            if !self.methods.is_empty() {
                writeln!(p, "")?;
                p.indent();
                for method in &self.methods {
                    p.write_indent()?;
                    p.write_idl(method)?;
                }
                p.dedent();
            }
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurExternCppFn {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::ExternFn) {
            let safety = if self.is_safe { "safe" } else { "unsafe" };
            let name = &self.name;
            write!(p, "{safety} fn {name}(")?;
            p.write_idl(&self.inputs)?;
            write!(p, ")")?;
            if self.output != crate::RustType::UNIT {
                write!(p, " -> ")?;
                p.write_idl(&self.output)?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurExternCppImpl {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::ExternImpl) {
            write!(p, "impl ")?;
            if self.tr.is_some() {
                p.write_idl(&self.tr)?;
                write!(p, " for ")?;
            }
            p.write_idl(&self.ty)?;
            writeln!(p, " {{")?;
            p.indent();
            for method in &self.methods {
                p.write_indent()?;
                let safety = if method.is_safe { "safe" } else { "unsafe" };
                write!(p, "{safety} ")?;
                p.write_idl(method)?;
                writeln!(p, ";")?;
            }
            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::LayoutPolicy {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            write!(p, "#")?;
            match self {
                Self::StackAllocated { size, align } => {
                    writeln!(p, "layout(size = {size}, align = {align});")?;
                }
                Self::Conservative { size, align } => {
                    writeln!(p, "layout_conservative(size = {size}, align = {align});")?;
                }
                Self::HeapAllocated => {
                    writeln!(p, "heap_allocate;")?;
                }
                Self::OnlyByRef => {
                    writeln!(p, "only_by_ref;")?;
                }
            }
        }
        Ok(())
    }
}

impl WriteIDL for crate::CppStackOwned {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let CppStackOwned {
                cpp_type: ty,
                size,
                align,
            } = self;
            writeln!(
                p,
                r#"#cpp_stack_owned "{ty}" (size = {size}, align = {align});"#
            )?;
        }
        Ok(())
    }
}
impl WriteIDL for crate::CppHeapAllocated {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let ty = &self.0;
            writeln!(p, r#"#cpp_heap_allocated "{ty}";"#)?;
        }
        Ok(())
    }
}
impl WriteIDL for crate::CppRef {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let ty = &self.0;
            writeln!(p, r#"#cpp_ref "{ty}";"#)?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurWellknownTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Debug => write!(p, "Debug"),
            Self::Drop => write!(p, "Drop"),
            Self::Unsized => write!(p, "Unsized"),
            Self::Copy => write!(p, "Copy"),
        }
    }
}

impl WriteIDL for crate::ZngurConstructor {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeConstructor) {
            let valid_tuple_repr = self
                .inputs
                .iter()
                .all(|(name, _)| name.parse::<usize>().is_ok());
            let (open, close) = if valid_tuple_repr {
                ("(", ")")
            } else {
                ("{", "}")
            };
            write!(p, "constructor {open}")?;
            for (i, (name, input)) in self.inputs.iter().enumerate() {
                if i > 0 && i < self.inputs.len() {
                    write!(p, ", ")?;
                }
                if !valid_tuple_repr {
                    write!(p, "{name}: ")?;
                }
                p.write_idl(input)?;
            }
            writeln!(p, "{close};")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurVariant {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeVariant) {
            let name = &self.name;
            write!(p, "variant {name} {{")?;
            let print_fields = !self.fields.is_empty() && p.enabled(&IDLItem::TypeVariantField);
            if print_fields || !self.exhaustive {
                writeln!(p, "")?;
            }
            p.indent();
            if !self.exhaustive {
                p.write_indent()?;
                writeln!(p, "non_exhaustive;")?;
            }

            if print_fields {
                for field in &self.fields {
                    p.write_indent()?;
                    p.write_idl(field)?;
                }
            }
            p.dedent();
            if print_fields || !self.exhaustive {
                p.write_indent()?;
            }
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurField {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeField) {
            let name = &self.name;
            let offset = self
                .offset
                .map_or("auto".to_string(), |size| size.to_string());
            write!(p, "field {name} (offset = {offset}, type = ")?;
            p.write_idl(&self.ty)?;
            writeln!(p, ");")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurMethodDetails {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeMethod) {
            let safety = if self.data.is_safe { "" } else { "unsafe " };
            write!(p, "{safety}")?;
            p.write_idl(&self.data)?;
            if let Some(use_path) = &self.use_path {
                let path = use_path.iter().join("::");
                let path = if use_path.first().is_some_and(|p| p == "crate") {
                    path
                } else {
                    format!("::{path}")
                };
                write!(p, " use {path}")?;
            }
            if let Some((ty, _)) = &self.deref {
                write!(p, " deref ")?;
                p.write_idl(ty)?;
            }
            if let Some(name) = &self.cpp_name {
                write!(p, " as {name}")?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurMethod {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        let name = &self.name;
        let receiver = match self.receiver {
            crate::ZngurMethodReceiver::Static => "",
            crate::ZngurMethodReceiver::Move => "self",
            crate::ZngurMethodReceiver::Ref(mu) => match mu {
                crate::Mutability::Not => "&self",
                crate::Mutability::Mut => "&mut self",
            },
        };
        let rec_sep = if self.inputs.len() > 0
            && !matches!(self.receiver, crate::ZngurMethodReceiver::Static)
        {
            ", "
        } else {
            ""
        };
        write!(p, "fn {name}")?;
        if self.generics.len() > 0 {
            write!(p, "<")?;
            for (i, generic) in self.generics.iter().enumerate() {
                p.write_idl(generic)?;
                if i < self.generics.len() {
                    write!(p, ", ")?;
                }
            }
            write!(p, ">")?;
        }
        write!(p, "({receiver}{rec_sep}")?;
        for (i, input) in self.inputs.iter().enumerate() {
            if i > 0 && i < self.inputs.len() {
                write!(p, ", ")?;
            }
            p.write_idl(input)?;
        }
        write!(p, ")")?;
        if self.output != crate::RustType::UNIT {
            write!(p, " -> ")?;
            p.write_idl(&self.output)?;
        }
        Ok(())
    }
}

#[test]
fn print_spec() {
    let spec = crate::ZngurSpec {
        types: vec![crate::ZngurType {
            ty: crate::RustType::Adt(crate::RustPathAndGenerics {
                path: vec!["crate".into(), "Example".into()],
                generics: vec![],
                named_generics: vec![],
            }),
            layout: Some(crate::LayoutPolicy::Conservative { size: 16, align: 8 }),
            wellknown_traits: vec![crate::ZngurWellknownTrait::Debug],
            exhaustive: true,
            constructor: None,
            variants: vec![],
            fields: vec![
                crate::ZngurField {
                    name: "foo".into(),
                    ty: crate::RustType::Primitive(crate::PrimitiveRustType::Uint(64)),
                    offset: Some(0),
                },
                crate::ZngurField {
                    name: "bar".into(),
                    ty: crate::RustType::Primitive(crate::PrimitiveRustType::Float(64)),
                    offset: None,
                },
            ],
            methods: vec![crate::ZngurMethodDetails {
                data: crate::ZngurMethod {
                    name: "foo_bar".into(),
                    generics: vec![],
                    receiver: crate::ZngurMethodReceiver::Ref(crate::Mutability::Not),
                    inputs: vec![],
                    output: crate::RustType::UNIT,
                    is_safe: true,
                },
                use_path: None,
                deref: None,
                cpp_name: Some("fooBar".into()),
            }],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        }],
        traits: indexmap::IndexMap::default(),
        funcs: vec![crate::ZngurFn {
            path: crate::RustPathAndGenerics {
                path: vec!["crate".into(), "do_w_example".into()],
                generics: vec![],
                named_generics: vec![],
            },
            inputs: vec![crate::RustType::Ref(
                crate::Mutability::Not,
                Box::new(crate::RustType::Adt(crate::RustPathAndGenerics {
                    path: vec!["crate".into(), "Example".into()],
                    generics: vec![],
                    named_generics: vec![],
                })),
            )],
            output: crate::RustType::Primitive(crate::PrimitiveRustType::Bool),
        }],
        extern_cpp_funcs: vec![],
        extern_cpp_impls: vec![],
        ..Default::default()
    };

    let mut buf = std::io::BufWriter::new(Vec::new());

    let mut printer = crate::printing::IDLPrinter::new(&mut buf);

    printer.write_idl(&spec).expect("write error");

    let bytes = buf.into_inner().expect("buffer flush error");

    let idl = String::from_utf8(bytes).expect("utf8 error");

    assert_eq!(
        "type crate::Example {
    #layout_conservative(size = 16, align = 8);
    wellknown_traits(Debug);

    field foo (offset = 0, type = u64);
    field bar (offset = auto, type = f64);

    fn foo_bar(&self) as fooBar;
}

fn crate::do_w_example(&crate::Example) -> bool;
",
        idl
    );
}
