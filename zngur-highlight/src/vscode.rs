const KEYWORDS: [(&'static str, &'static str); 21] = [
    // see zngur-parser/src/lib.rs -> impl Token -> fn ident_or_kw
    ("keyword.operator.zngur.as", "as"),
    ("storage.modifier.zngur.async", "async"),
    ("storage.type.zngur.dyn", "dyn"),
    ("entity.name.section.zngur.mod", "mod"),
    ("storage.type.zngur.type", "type"),
    ("storage.type.zngur.trait", "trait"),
    ("storage.modifier.zngur.crate", "crate"),
    ("entity.name.function.zngur.fn", "fn"),
    ("storage.modifier.zngur.mut", "mut"),
    ("storage.modifier.zngur.const", "const"),
    ("storage.modifier.zngur.use", "use"),
    ("keyword.control.zngur.for", "for"),
    ("storage.modifier.zngur.extern", "extern"),
    ("storage.type.zngur.impl", "impl"),
    ("keyword.other.zngur.import", "import"),
    ("keyword.other.zngur.merge", "merge"),
    ("keyword.control.zngur.if", "if"),
    ("keyword.control.zngur.else", "else"),
    ("keyword.control.zngur.match", "match"),
    ("storage.modifier.zngur.safe", "safe"),
    ("storage.modifier.zngur.unsafe", "unsafe")
];
const SHARPS: &'static [&'static str; 11] = [
    "#unstable",
    "#layout",
    "#layout_conservative",
    "#only_by_ref",
    "#heap_allocated",
    "#cpp_value",
    "#cpp_heap_allocated",
    "#cpp_ref",
    "#cpp_stack_owned",
    "#cpp_additional_includes",
    "#convert_panic_to_exception"
];
const RUST_SCALAR: [&'static str; 15] = [
    "u8", "u16", "u32", "u64", "u128", "usize",
    // Note: isize not supported in zngur-parser/src/lib.rs$rust_type$scalar
    "i8", "i16", "i32", "i64", "i128",// "isize",
    "bool", "str", "char"
];
const PSEUDO_KEYWORDS: [&'static str] = [
    "wellknown_traits",
    "variant"
];

type Name = Rc<str>;
type Regex = Rc<str>;
type StartStop = (Regex, Regex);
#[derive(Clone)]
struct Scope {
    name: Name,
    // fileTypes are .zng, .zngur
    folding_start_stop: Option<StartStop>,
    patterns: Vec<Rc<Rule>>,
    repository: HashMap<Name, Rc<Rule>>
}
type Rules = Vec<Rc<Rule>>;
enum Rule {
    Include(Name),
    Patterns(Rules),
    // Regex is `match`, Rules is `patterns`
    Match(Name, Regex, Rules),
    // StartStop is (begin, end)
    InclBeginEnd(Name, StartStop, Rules),
    // name is contentName
    ExclBeginEnd(Name, StartStop, Rules)
}
impl Scope {
    fn new<N>(name: N, folding: Option<StartStop>,
                        patterns: Vec<Rc<Rule>>,
                        repository: HashMap<Name, Rc<Rule>>
            ) -> Rc<Self>
                where for<'a> &'a N: Into<Name> {
        let name: Name = name.into();
        Rc::new(Scope {
            name, folding_start_stop: folding,
            patterns, repository
        })
    }
}
impl Rule {
    fn include<N>(name: N) -> Rc<Self>
                where N: Into<Name> {
        let name: Name = name.into();
        Rc::new(Rule::Include(name))
    }
    fn patterns(rules: Rules) -> Rc<Self> {
        Rc::new(Rule::Patterns(rules))
    }
    fn match_<N, M>(name: N, match_re: M, rules: Rules) -> Rc<Self>
                where N: Into<Name>,
                      M: Into<Regex> {
        let name: Name = name.into();
        let match_re: Regex = match_re.into();
        Rc::new(Rule::Match(name, match_re, rules))
    }
    fn range_incl<N, B, E>(name: N, range: (B, E), rules: Rules) -> Rc<Self>
                where N: Into<Name>,
                      B: Into<Regex>,
                      E: Into<Regex> {
        let name: Name = name.into();
        let (start, stop): StartStop = range.into();
        Rc::new(Rule::InclBeginEnd(name, (start, stop), rules))
    }
    fn range_excl<N, B, E>(name: N, range: (B, E), rules: Rules) -> Rc<Self>
                where N: Into<Name>,
                      B: Into<Regex>,
                      E: Into<Regex> {
        let name: Name = name.into();
        let (start, stop): StartStop = range.into();
        Rc::new(Rule::ExclBeginEnd(name, (start, stop), rules))
    }
}

// Chumsky is made up of parser combinators,
//      like Haskell's ReadP or Attoparsec
// grammar root: src/lib.rs$file_parser:2624
// https://github.com/hkalbasi/zngur/blob/2fd07e1ad8e709ed3466b2193a1c7f853b869760/zngur-parser/src/lib.rs#L2624
const GRAMMAR: LazyLock<Scope> = LazyLock::new(|| {
    let comment = Rule::match_(
        "comment.line.double-slash.zngur",
        "//[^\n]*$" /* match re */, vec![]
    );
    let plain_string = Rule::match_(
        "string.quoted.double.zngur",
        "\"(?:[^\"]|\\\")**\"", vec![]
    );
    let raw_string = Rule::range_incl(
        "string.quoted.other.raw.zngur",
        ("(#+)\"", "\"\\1"), vec![]
    );
    let keywords: Rules = KEYWORDS.iter()
        .map(|(name, kwd)| {
            Rule::match_(name, kwd, Vec::new())
        }).collect();
    // don't fold the root
    let root = Scope::new("source.zngur", None,
        vec![Rule::include("item")],
        HashMap::<Name, Rc<Rule>>::from([
            ("comment", comment.clone()),
            ("string", Rule::patterns(vec![
                plain_string.clone(),
                raw_string.clone()
            ])),
            ("item", Rule::patterns(vec![
                Rule::include("unstable_feature"),
                Rule::range_incl(
                    "meta.module.zngur.mod",
                    ("mod", ";"),
                    
                )
            ]))
        ])
    );
});

trait DumpAsJson {
    fn dump_as_json(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result<()>;
}
impl DumpAsJson for Scope {
    fn dump_as_json(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result<()> {
        
    }
}

