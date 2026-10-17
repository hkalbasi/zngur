use convert_case::{Case, Casing};
use rustc_attr_parsing::template;
use rustc_span::Span;

use crate::{
    attrs::{
        AUTOZNG_NAMESPACE, Attr, AttrPath,
        parse::{
            context::AcceptContext,
            parser::{ArgParser, AttrtibuteParser},
        },
    },
    ctx::AutozngContext,
    metadata::{ExportName, IntoMetadataKey},
};

pub trait RenameTarget: IntoMetadataKey {
    fn item_name(&self) -> String;
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

/// Applies
#[derive(Debug, Clone)]
pub struct Rename {
    path: AttrPath,
    actions: Vec<RenameAction>,
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

    fn for_child(&self, item: &dyn RenameTarget) -> Option<Self> {
        // if this is a rename_all aimed at children and a re-nameable items
        if self.path() == RENAME_ALL_PATH && matches!(self.target, RenameTargetType::Children) {
            // redirect target to This for children
            Some(Rename {
                path: self.path.clone(),
                actions: self.actions.clone(),
                target: RenameTargetType::This,
                span: self.span,
            })
        } else {
            None
        }
    }

    fn visit(&self, item: &dyn RenameTarget, ctx: &AutozngContext) {
        // if item re-nameable and the attr targets `This`
        if matches!(self.target, RenameTargetType::This) {
            // apply actions in order
            let export_name = self
                .actions
                .iter()
                .fold(item.item_name(), |name, action| action.apply(&name));
            // set the new name, remembering the attribute that produced it
            ctx.with_metadata_mut(item, |mut metadata| {
                metadata.export_name = Some(ExportName {
                    name: export_name,
                    span: self.span,
                });
            });
        }
    }
}

#[derive(Debug, Default)]
pub struct RenameParser {
    path: Option<AttrPath>,
    actions: Option<Vec<RenameAction>>,
    target: Option<RenameTargetType>,
    span: Option<Span>,
}

const RENAME_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "rename"]);
const RENAME_ALL_PATH: AttrPath = AttrPath::from_static(&[AUTOZNG_NAMESPACE, "rename_all"]);
const RENAME_PATH_TARGETS: &[(AttrPath, RenameTargetType)] = &[
    (RENAME_PATH, RenameTargetType::This),
    (RENAME_ALL_PATH, RenameTargetType::Children),
];

impl AttrtibuteParser for RenameParser {
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
                    r#"suffix = "suffix""#
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

    fn finalize(self, target: &<Self::Attr as Attr>::Target) -> Option<Self::Attr> {
        if let RenameParser {
            path: Some(path),
            actions: Some(actions),
            target: Some(target),
            span: Some(span),
        } = self
        {
            Some(Rename {
                path,
                actions,
                target,
                span,
            })
        } else {
            None
        }
    }
}

mod diagnotics {

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
                .map(|action| -> Result<RenameAction, ()> {
                    let Some((action, value)) = cx.expect_name_value(action, action.span(), None)
                    else {
                        return Err(());
                    };

                    let Some(action) = RENAME_ACTION_NAMES.iter().find_map(|(action_ty, name)| {
                        if action.value == *name {
                            Some(action_ty)
                        } else {
                            None
                        }
                    }) else {
                        cx.dcx.emit_err(diagnotics::UnknownRenameAction {
                            span: action.span,
                            name: action.value,
                            valid_actions: diagnotics::ValidRenameActions(
                                RENAME_ACTION_NAMES
                                    .iter()
                                    .map(|(_, a)| -> &'static str { a })
                                    .collect(),
                            ),
                        });
                        return Err(());
                    };

                    let Some(value) = cx.expect_string_literal(value) else {
                        return Err(());
                    };

                    match action {
                        RenameActionType::Rename => {
                            // it makes not send to apply a single name to all items
                            if *path == RENAME_ALL_PATH {
                                cx.dcx
                                    .struct_err("unsupported rename action in `rename_all`")
                                    .emit();
                                return Err(());
                            }

                            Ok(RenameAction::Rename(value.value))
                        }
                        RenameActionType::ConvertCase => {
                            let Some(case) =
                                RenameParser::expect_case(&value.value, value.span, cx)
                            else {
                                return Err(());
                            };

                            Ok(RenameAction::ConvertCase(case))
                        }
                        RenameActionType::Prefix => Ok(RenameAction::Prefix(value.value)),
                        RenameActionType::Suffix => Ok(RenameAction::Suffix(value.value)),
                    }
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
            cx.dcx.emit_err(diagnotics::UnknownConvertCase {
                span,
                case: name,
                valid_cases: diagnotics::ValidConvertCases(valid_cases()),
            });
            return None;
        };
        Some(case)
    }
}
