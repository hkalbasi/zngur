use convert_case::{Case, Casing};
use rustc_attr_parsing::template;
use rustc_public::CrateDef;
use rustc_public::ty::{AssocKind, Ty};
use rustc_span::Span;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttributeParser},
        },
    },
    ctx::AutozngContext,
    error::{emit_error_for_span, emit_warning_for_span},
    metadata::{ExportName, IntoMetadataKey, MetadataKey},
    rustc_ext::RustcPublicTyExt,
};

pub trait RenameTarget: IntoMetadataKey {
    fn item_name(&self) -> String;

    /// The type whose inherent methods a `rename(method = "...")` can target, when this item can
    /// carry one: an ADT (its own type) or a type alias (the aliased type).
    fn method_carrier(&self) -> Option<Ty> {
        None
    }

    /// Whether this item is a type alias. An alias is not emitted itself, so it only accepts
    /// `rename(method = "...")`.
    fn is_type_alias(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RenameActionType {
    ConvertCase,
    Prefix,
    Suffix,
    Rename,
}

const RENAME_ACTION_NAMES: &[(RenameActionType, &str)] = &[
    (RenameActionType::ConvertCase, "case"),
    (RenameActionType::Prefix, "prefix"),
    (RenameActionType::Suffix, "suffix"),
    (RenameActionType::Rename, "name"),
];

#[derive(Debug, Clone)]
pub enum RenameAction {
    ConvertCase(convert_case::Case<'static>),
    Prefix(String),
    Suffix(String),
    Rename(String),
}

impl RenameAction {
    pub fn apply(&self, name: &str) -> String {
        match self {
            Self::ConvertCase(case) => name.to_case(*case),
            Self::Prefix(prefix) => format!("{prefix}{name}"),
            Self::Suffix(suffix) => format!("{name}{suffix}"),
            Self::Rename(new_name) => new_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RenameTargetType {
    This,
    Children,
}

/// One renaming carried by a `rename` attribute: the item itself, one of its methods, or the
/// methods of a trait implemented by it.
#[derive(Debug, Clone)]
pub struct RenameSpec {
    /// `Some(path)` renames the methods of that trait; `method` may narrow it to one method.
    trait_path: Option<String>,
    /// `Some(name)` targets that method; `None` targets the item itself (or every method, with a
    /// `trait_path`).
    method: Option<String>,
    actions: Vec<RenameAction>,
}

impl RenameSpec {
    fn apply(&self, name: &str) -> String {
        self.actions
            .iter()
            .fold(name.to_owned(), |name, action| action.apply(&name))
    }
}

/// A `rename(trait_path = "...")` recorded on an ADT, applied when a trait method is rendered on
/// it.
#[derive(Debug, Clone)]
pub struct TraitRenameSpec {
    /// The trait path exactly as it appears in the `.zng` (e.g. `::std::convert::From<u32>`).
    pub trait_path: String,
    /// `Some(name)` narrows the rename to that method of the trait.
    pub method: Option<String>,
    pub actions: Vec<RenameAction>,
    /// Span of the attribute, for diagnostics.
    pub span: Span,
}

/// Applies `rename` / `rename_all` actions.
#[derive(Debug, Clone)]
pub struct Rename {
    path: AttrPath,
    /// One attribute may carry several renames (e.g. the type itself and one of its methods);
    /// duplicate `rename` attributes on one item are merged into this list.
    specs: Vec<RenameSpec>,
    target: RenameTargetType,
    /// Span of the `#[autozng::rename(...)]` attribute, kept so it can be recorded alongside the
    /// export name and used for a `renamed here` diagnostic.
    span: Span,
}

impl Attr for Rename {
    type Target = dyn RenameTarget;

    fn path(&self) -> AttrPath {
        self.path.clone()
    }

    fn for_child(&self, _item: &dyn RenameTarget) -> Option<Self> {
        // if this is a rename_all aimed at children and a re-nameable items
        if self.path() == RENAME_ALL_PATH && matches!(self.target, RenameTargetType::Children) {
            // redirect target to This for children
            Some(Rename {
                path: self.path.clone(),
                specs: self.specs.clone(),
                target: RenameTargetType::This,
                span: self.span,
            })
        } else {
            None
        }
    }

    fn merge(&mut self, other: Self) {
        // Two `rename` attributes on one item keep both specs: one may rename the item itself
        // while the other renames a method, for example.
        self.specs.extend(other.specs);
    }

    fn visit(&self, item: &dyn RenameTarget, ctx: &AutozngContext) {
        if !matches!(self.target, RenameTargetType::This) {
            return;
        }
        for spec in &self.specs {
            if let Some(trait_path) = &spec.trait_path {
                let Some(carrier) = item.method_carrier() else {
                    emit_error_for_span(
                        self.span,
                        "autozng: `trait_path = \"...\"` is only valid on an ADT or a type alias",
                    );
                    continue;
                };
                let Some(adt) = carrier.base_ty().adt_def() else {
                    emit_error_for_span(
                        self.span,
                        "autozng: `trait_path = \"...\"` needs an ADT or a type alias to an ADT",
                    );
                    continue;
                };
                ctx.add_trait_rename(
                    &adt,
                    TraitRenameSpec {
                        trait_path: trait_path.clone(),
                        method: spec.method.clone(),
                        actions: spec.actions.clone(),
                        span: self.span,
                    },
                );
                continue;
            }
            let Some(method) = &spec.method else {
                if item.is_type_alias() {
                    emit_error_for_span(
                        self.span,
                        "autozng: `rename` on a type alias needs `method = \"...\"`: the alias \
                         itself is not emitted",
                    );
                    continue;
                }
                let export_name = spec.apply(&item.item_name());
                ctx.with_metadata_mut(item, |mut metadata| {
                    metadata.export_name = Some(ExportName {
                        name: export_name,
                        span: self.span,
                    });
                });
                continue;
            };
            let Some(carrier) = item.method_carrier() else {
                emit_error_for_span(
                    self.span,
                    "autozng: `method = \"...\"` is only valid on an ADT or a type alias",
                );
                continue;
            };
            let Some((def_id, method_name)) = inherent_method(carrier, method) else {
                emit_warning_for_span(
                    self.span,
                    format!("autozng: no inherent method `{method}` on `{carrier}` to rename"),
                );
                continue;
            };
            let export_name = spec.apply(&method_name);
            ctx.with_metadata_mut(&MetadataKey::from_public_def_id(def_id), |mut metadata| {
                metadata.export_name = Some(ExportName {
                    name: export_name,
                    span: self.span,
                });
            });
        }
    }
}

/// Find an inherent method by name on the type `ty` names, returning its definition and simple
/// name.
fn inherent_method(ty: Ty, name: &str) -> Option<(rustc_public::DefId, String)> {
    let adt = ty.base_ty().adt_def()?;
    for impl_def in adt.inherent_impls() {
        for assoc_item in impl_def.associated_items() {
            let AssocKind::Fn {
                name: method_name, ..
            } = &assoc_item.kind
            else {
                continue;
            };
            if method_name == name {
                return Some((assoc_item.def_id.def_id(), method_name.clone()));
            }
        }
    }
    None
}

#[derive(Debug, Default)]
pub struct RenameParser {
    path: Option<AttrPath>,
    actions: Option<Vec<RenameAction>>,
    /// `method = "name"`: target that method instead of the item itself.
    method: Option<String>,
    /// `trait = "path"`: target the methods of that trait (optionally narrowed by `method`).
    trait_path: Option<String>,
    target: Option<RenameTargetType>,
    span: Option<Span>,
}

const RENAME_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "rename"]);
const RENAME_ALL_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "rename_all"]);
const RENAME_PATH_TARGETS: &[(AttrPath, RenameTargetType)] = &[
    (RENAME_PATH, RenameTargetType::This),
    (RENAME_ALL_PATH, RenameTargetType::Children),
];

impl AttributeParser for RenameParser {
    type Attr = Rename;

    const ATTRIBUTES: crate::attrs::parse::parser::AcceptAttrMap<Self> = &[
        (
            RENAME_PATH,
            template!(
                List: &[
                    r#""new_name""#,
                    r#"name = "new_name""#,
                    r#"case = "case""#,
                    r#"prefix = "pre"#,
                    r#"suffix = "suffix""#,
                    r#"method = "method_name""#,
                    r#"trait_path = "trait_path""#
                ],
                "https://link/to/docs"
            ),
            RenameParser::parse,
        ),
        (
            RENAME_ALL_PATH,
            template!(
                List: &[
                    r#"case = "case""#,
                    r#"prefix = "pre"#,
                    r#"suffix = "suffix""#
                ]
            ),
            RenameParser::parse,
        ),
    ];

    fn finalize(self, _target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        if let RenameParser {
            path: Some(path),
            actions: Some(actions),
            method,
            trait_path,
            target: Some(target),
            span: Some(span),
        } = self
        {
            // Renaming every method of a trait to the same name would produce duplicate C++
            // members; a `trait` rename must keep the methods distinct.
            if trait_path.is_some()
                && method.is_none()
                && actions
                    .iter()
                    .any(|action| matches!(action, RenameAction::Rename(_)))
            {
                emit_error_for_span(
                    span,
                    "autozng: `name` cannot rename every method of a trait; add `method = \"...\"` \
                     or use `case`/`prefix`/`suffix`",
                );
                return None;
            }
            Some(Rename {
                path,
                specs: vec![RenameSpec {
                    trait_path,
                    method,
                    actions,
                }],
                target,
                span,
            })
        } else {
            None
        }
    }
}

mod diagnostics {

    use rustc_errors::into_diag_arg_using_display;
    use rustc_macros::Diagnostic;
    use rustc_span::Span;

    pub struct ValidConvertCases(pub Vec<&'static str>);

    impl std::fmt::Display for ValidConvertCases {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            for (i, case) in self.0.iter().enumerate() {
                f.write_str("\"")?;
                f.write_str(case)?;
                f.write_str("\"")?;
                if i < self.0.len() {
                    f.write_str(", ")?;
                }
            }
            Ok(())
        }
    }

    into_diag_arg_using_display!(ValidConvertCases);

    #[derive(Diagnostic)]
    #[diag("unknown convert case name `{$case}`")]
    #[help("use a case from: {$valid_cases}")]
    pub struct UnknownConvertCase<'a> {
        #[primary_span]
        pub span: Span,
        pub case: &'a str,
        pub valid_cases: ValidConvertCases,
    }

    pub struct ValidRenameActions(pub Vec<&'static str>);

    impl std::fmt::Display for ValidRenameActions {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            for (i, action) in self.0.iter().enumerate() {
                f.write_str(action)?;
                if i < self.0.len() {
                    f.write_str(", ")?;
                }
            }
            Ok(())
        }
    }

    into_diag_arg_using_display!(ValidRenameActions);

    #[derive(Diagnostic)]
    #[diag("unknown rename action `{$name}`")]
    #[help("use am action from: {$valid_actions}")]
    pub struct UnknownRenameAction<'a> {
        #[primary_span]
        pub span: Span,
        pub name: &'a str,
        pub valid_actions: ValidRenameActions,
    }
}

impl RenameParser {
    fn parse(&mut self, cx: &mut AcceptContext, args: &ArgParser) {
        let path = cx.attr_path;
        // if re-nameable
        let Some(target) = RENAME_PATH_TARGETS
            .iter()
            .find_map(|(p, target)| if p == path { Some(target) } else { None })
            .copied()
        else {
            cx.dcx
                .struct_span_err(
                    cx.span,
                    format!("BUG! unknown rename target for path: `{path}`"),
                )
                .emit();
            return;
        };

        self.path = Some(path.clone());
        self.target = Some(target);
        self.span = Some(cx.span);

        let Some(args) = cx.expect_list(args, cx.span) else {
            return;
        };

        if let Some(lit) = args.as_single().and_then(|arg| arg.as_lit()) {
            // direct str literal => rename only on RENAME_PATH
            if *path == RENAME_ALL_PATH {
                cx.expected_name_value(lit.span, None);
                return;
            }
            let Some(name) = cx.expect_string_literal(lit) else {
                return;
            };
            self.actions = Some(vec![RenameAction::Rename(name.value)]);
        } else {
            let actions = args
                .mixed()
                .filter_map(|arg| -> Option<Result<RenameAction, ()>> {
                    let Some((action, value)) = cx.expect_name_value(arg, arg.span(), None) else {
                        return Some(Err(()));
                    };

                    // `method = "name"` selects a method of the carrier instead of the item itself.
                    if action.value == "method" {
                        if *path == RENAME_ALL_PATH {
                            cx.dcx
                                .struct_err("`method` is not supported in `rename_all`")
                                .emit();
                            return Some(Err(()));
                        }
                        let Some(value) = cx.expect_string_literal(value) else {
                            return Some(Err(()));
                        };
                        self.method = Some(value.value);
                        return None;
                    }

                    // `trait_path = "path"` targets the methods of that trait on the carrier.
                    if action.value == "trait_path" {
                        if *path == RENAME_ALL_PATH {
                            cx.dcx
                                .struct_err("`trait_path` is not supported in `rename_all`")
                                .emit();
                            return Some(Err(()));
                        }
                        let Some(value) = cx.expect_string_literal(value) else {
                            return Some(Err(()));
                        };
                        self.trait_path = Some(value.value);
                        return None;
                    }

                    let Some(action) = RENAME_ACTION_NAMES.iter().find_map(|(action_ty, name)| {
                        if action.value == *name {
                            Some(action_ty)
                        } else {
                            None
                        }
                    }) else {
                        cx.dcx.emit_err(diagnostics::UnknownRenameAction {
                            span: action.span,
                            name: action.value,
                            valid_actions: diagnostics::ValidRenameActions(
                                RENAME_ACTION_NAMES
                                    .iter()
                                    .map(|(_, a)| -> &'static str { a })
                                    .collect(),
                            ),
                        });
                        return Some(Err(()));
                    };

                    let Some(value) = cx.expect_string_literal(value) else {
                        return Some(Err(()));
                    };

                    Some(match action {
                        RenameActionType::Rename => {
                            // it makes not send to apply a single name to all items
                            if *path == RENAME_ALL_PATH {
                                cx.dcx
                                    .struct_err("unsupported rename action in `rename_all`")
                                    .emit();
                                return Some(Err(()));
                            }

                            Ok(RenameAction::Rename(value.value))
                        }
                        RenameActionType::ConvertCase => {
                            let Some(case) =
                                RenameParser::expect_case(&value.value, value.span, cx)
                            else {
                                return Some(Err(()));
                            };

                            Ok(RenameAction::ConvertCase(case))
                        }
                        RenameActionType::Prefix => Ok(RenameAction::Prefix(value.value)),
                        RenameActionType::Suffix => Ok(RenameAction::Suffix(value.value)),
                    })
                })
                .collect::<Vec<_>>();

            // all arguments are good or the parser is invalid
            self.actions = actions.into_iter().collect::<Result<Vec<_>, _>>().ok();
        }
    }

    fn expect_case(name: &str, span: Span, cx: &mut AcceptContext) -> Option<Case<'static>> {
        const CASES: &[(&str, Case)] = &[
            ("snake_case", Case::Snake),
            ("CONSTANT", Case::Constant),
            ("UPPER_SNAKE", Case::Constant),
            ("Ada", Case::Ada),
            ("Capital_Snake", Case::Ada),
            ("flat", Case::Flat),
            ("UPPERFLAT", Case::UpperFlat),
            ("Pascal", Case::Pascal),
            ("UpperCamel", Case::Pascal),
            ("camelCase", Case::Camel),
        ];
        let valid_cases = || -> Vec<&'static str> {
            CASES
                .iter()
                .map(|(s, _)| -> &'static str { s })
                .collect::<Vec<_>>()
        };

        let Some(case) = CASES.iter().find_map(|(case_name, case)| {
            if *case_name == name {
                Some(*case)
            } else {
                None
            }
        }) else {
            cx.dcx.emit_err(diagnostics::UnknownConvertCase {
                span,
                case: name,
                valid_cases: diagnostics::ValidConvertCases(valid_cases()),
            });
            return None;
        };
        Some(case)
    }
}
