use itertools::Itertools;
use std::path::Path;
use std::borrow::Cow;
use std::env;
use std::sync::LazyLock;
use std::collections::HashMap;

mod tree_sitter;
use tree_sitter::emit_tree_sitter;

const KEYWORDS: [&'static str; 21] = [
    // see zngur-parser/src/lib.rs -> impl Token -> fn ident_or_kw
    "as",
    "async",
    "dyn",
    "mod",
    "type",
    "trait",
    "crate",
    "fn",
    "mut",
    "const",
    "use",
    "for",
    "extern",
    "impl",
    "import",
    "merge",
    "if",
    "else",
    "match",
    "safe",
    "unsafe"
];
const SHARPS: [&'static str; 11] = [
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
const RUST_SCALAR: [&'static str; 14] = [
    "u8", "u16", "u32", "u64", "u128", "usize",
    // Note: isize not supported in zngur-parser/src/lib.rs$rust_type$scalar
    "i8", "i16", "i32", "i64", "i128",// "isize",
    "bool", "str", "char"
];
const PSEUDO_KEYWORDS: [&'static str; 2] = [
    "wellknown_traits",
    "variant"
];

fn raw_string_regex(n: usize) -> String {
    // #{n}"(?:(?!"#{n}).)+"#{n}
    format!("(?:#{{{n}}}(?:(?!\"#{{{n}}}).)+\"#{{{n}}}))")
}

// zngur-parser/src/lib.rs$lexer()
fn regexes(n: usize) -> HashMap<&'static str, Cow<'static, str>> {
    HashMap::from([
        ("plain string", Cow::Borrowed("\"[^\"]*\"")),
        ("raw string", Cow::Owned(
            (1..=n).map(raw_string_regex).join("|")
        )),
        ("symbol", Cow::Borrowed(
            "->|=>|<|>|\\[|\\]|[(){}]|::|[:&*#+=?,;|_.!]"
        )),
        ("keyword", Cow::Owned(KEYWORDS.iter().join("|"))),
        // a usize
        ("number", Cow::Borrowed("0|[1-9][0-9]*")),

        ("comment", Cow::Borrowed("//[^\\n]*")),
        // so... this's actually unicode (XID_Start XID_Continue*),
        //          but we're only parsing this for highlighting,
        //          so just parse extra
        ("ident", Cow::Owned({
            let is_not_symbol =
                "(?=[^=<>(){}\\[\\];|!:&?,.*#+-\" \\n\\t\\r])(?!//)";
            let is_not_keyword: String =
                // seperate the keywords by length,
                //    so that each lookahead can be the same size
                KEYWORDS.iter().map(|kwd| (kwd.len(), kwd))
                    // from itertools
                    .into_group_map()
                    .into_values()
                    .map(|same_len_kwds| {
                        format!("(?!{})",
                            same_len_kwds.into_iter()
                                .join("|")
                        )
                    })
                    .collect();
            let is_not_number = "(?![0-9])";
            let ident = format!(
                "{is_not_number}({is_not_symbol}{is_not_keyword}.)+"
            );
            ident
        }))
    ])
}

type EmitError = Result<(), String>;
static RAW_STRING_REGEXES: LazyLock<Vec<String>> = LazyLock::new(|| {
    (1..=20usize).map(raw_string_regex)
        .collect_vec()
});
static REGEXES: LazyLock<HashMap<&'static str, Cow<str>>> = LazyLock::new(|| {
    regexes(20)
});

fn main() {
    let mut output_folder = String::from(".");
    let mut emits = HashMap::<
                        &'static str,
                        for<'a> fn(&'a str) -> EmitError
                    >::new();
    for arg in env::args().dropping(1) {
        match arg.as_str() {
            "--highlight-js" => {
                emits.insert("highlight.js", emit_highlight_js);
            },
            "--tree-sitter" => {
                emits.insert("tree sitter", emit_tree_sitter);
            },
            /*
            "--vscode-extension" => {
                emits.insert("vscode extension", emit_vscode);
            },*/
            _ => {
                output_folder = arg;
            }
        }
    }
    let output_folder_str = output_folder.as_ref();

    let errors =
        emits.into_iter()
        .filter_map(|(name, emit): (&'static str, for<'a> fn(&'a str) -> EmitError)| -> Option<(&str, String)> {
            emit(output_folder_str).err()
                .map(|err| (name, err))
        })
        .collect_vec();
    if errors.len() > 0 {
        panic!("{}",
            errors.into_iter()
                .map(|(name, err)| {
                    format!("Error emitting {name}: {}", err)
                }).join("\n")
            );
    }
}

fn emit_highlight_js(folder: &str) -> Result<(), String> {
    let mut regex_map = LazyLock::force(&REGEXES).clone();
    let ident_pat = "\\p{XID_Start}\\p{XID_Continue}*";
    regex_map.insert("ident", Cow::Borrowed(ident_pat));
    regex_map.remove("raw string");
    regex_map.remove("plain string");
    regex_map.remove("comment");
    let tokenizer =
        format!("(?:{})", regex_map.values().join(")|(?:"));

    let file = format!(
// looking at https://github.com/highlightjs/highlightjs-dylan/blob/master/src/dylan.js
// Also https://highlightjs.readthedocs.io/en/latest/language-guide.html
// Also https://github.com/highlightjs/highlight.js/blob/main/src/lib/modes.js
r##"
/*
Language: Zngur
Description: Zngur file definition for highlight.js
Author: cplir-c <31234188+cplir-c@users.noreply.github.com>
Category: Bindings Generation
Website: https://hkalbasi.github.io/zngur/
*/

module.exports = function(hljs) {{
    const KEYWORDS = {{
        $pattern: /{tokenizer}/u
        keyword: [{}],
        sharps: "{}",
        ident: /{ident_pat}/u,
        number: /0|[1-9][0-9]+/,
        _relevance_only: "{}"
    }};
    const ZNGUR_CODE = {{
        case_insensitive: false,
        scope: 'top-level',
        keywords: KEYWORDS,
        contains: [
            hljs.C_LINE_COMMENT_MODE,
            // Raw strings
            {},
            // strings
            hljs.QUOTE_STRING_MODE,
            
        ]
    }};
    return ZNGUR_CODE;
}}
"##,
    KEYWORDS.iter().map(|x| format!("\"{x}\"")).join(", "),
    // sharps; the | is for language-specificity on the right
    SHARPS.map(|kwd| format!("{}|{}", kwd, kwd.len() / 6 + 1))
        .join(" "),
    // "relevance-only" keywords:
    PSEUDO_KEYWORDS.iter().join(" "),
    // raw strings
    // We need the 1 to 20 part if we don't want to use back-references
    (1..=20).map(|n| {
        format!(
r##"{{
    scope: 'raw string',
    begin: /r#{{{n}}}"/,
    end: /"#{{{n}}}/,
    contains: [hljs.BACKSLASH_ESCAPE]
}}"##)
    }).join(", ")
    
    ); // let file = format!...
    
    let path = Path::new(folder).join(Path::new("zngur.js"));
    std::fs::write(&path, file)
        .map_err(|e| format!("{}", e))
}
