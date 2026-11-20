use std::collections::HashMap;
// join()
use itertools::Itertools;

use std::path::Path;
use crate::KEYWORDS;

type Parser = String;
type Parsers = HashMap<&'static str, Parser>;

fn make_grammar_js() -> String {
    format!(
r##"
/**
 * @file A Parser for the Zngur C++/Rust bindings language
 * @author cplir-c 31234188+cplir-c@users.noreply.github.com
 * @license Dual Apache 2.0 and MIT license
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

export default grammar({{
    name: "tree-sitter-zngur",
    extras: $ => [ /\s/, $.comment ],
    reserved: {{
        keywords: ['{keywords}'],
    }},
    rules: {{
        root: $ => repeat($.item),
        item: $ => choice(
            $.unstable_feature, // 1
            $.module, // 1
            $.type_item, // 1
            $.trait_item, // 1
            $.extern_cpp_item, // 1
            $.fn_item, // 1
            $.additional_include_item, // 1
            $.import_item, // 1
            $.module_import_item, // 1
            $.alias, // 1
            $.conditional_item // TODO
        ),
        comment: $ => seq("//", /[^\n]*/),
        identifier:
            $ => new RustRegex('\p{{XID_Start}}\p{{XID_Continue}}*'),
        ident: $ => alias($.identifier, $.ident),

        // ... cancel this, we're using Rust-side text templates
        {conditional_item_},

        unstable_feature: $ => seq(
            '#', 'unstable',
            // ident can be 'cfg_match', 'cfg_if', or 'template_types'
            '(', $.ident, ')'
        ),

        module_import_item: $ => seq(
            'import', $.plain_string, ';'
        ),

        import_item: $ => seq(
            'merge', $.plain_string, ';' // TODO
        ),

        additional_include_item: $ => seq(
            '#',
            choice(
                seq(
                    'cpp_additional_includes',
                    $.rawable_string // TODO
                ),
                'convert_panic_to_exception'
            )
        ),

        fn_item: $ => seq(
            optional(choice('safe', 'unsafe')),
            $.function_signature, ';' // TODO
        ),

        extern_cpp_item: $ => {{
            const safety = choice('safe', 'unsafe');
            const function = seq(
                safety, $.method_signature, ';' // 1, TODO
            );
            // impl RustTrait for RustType {{ safety method; ... }}
            const impl_block = seq(
                'impl',
                optional(seq(
                    $.rust_trait, // TODO
                    'for',
                )),
                $.rust_type, // TODO
                '{{',
                repeat(seq(
                    safety, $.method_signature, ';' // 1, TODO
                )),
                '}}',
            );
            return seq(
                'extern',
                '"C++"',
                '{{',
                repeat(choice(function, impl_block)),
                '}}'
            );
        }},

        trait_item: $ => seq(
            'trait',
            $.rust_trait, // TODO
            '{{', repeat(seq($.method, ';')), '}}'
        ),

        type_item: $ => seq(
            'type',
            optional(seq(
                '<',
                $.ident, repeat(seq(',', $.ident)), optional(','),
                '>'
            )),
            $.rust_type, // TODO
            '{{', repeat($.inner_type_item), '}}' // TODO
        ),

        module: $ => seq(
            'mod',
            $.path, // 1
            $.braced_items // 1
        ),
        braced_items: $ => seq(
            // 1, 1, 1
            '{{', repeat($.item), '}}'
        ),
        // lib.rs:3367
        path: $ => {{
            const start = optional(seq(
                choice(
                    '::',
                    seq(
                        'crate',
                        '::'
                    )
                )
            ));

            return seq(
                start, // 1
                $.ident, // TODO
                repeat(seq('::', $.ident)) // 1, TODO
            );
        }},
        // lib.rs:606
        // e.g. ? `use ::std::boxed::Box as Boxed;`
        alias: $ => seq(
            'use',
            $.path, // 1
            'as',
            $.ident, // 1
            ';'
        ),
        

    }}
}});
"##,
        keywords = KEYWORDS.iter().join("', '"),
        conditional_item_ = conditional_item().into_iter()
            .map(|(name, body)| {
                format!("{name}: {body}")
            })
            .join(", ")
    )
}

pub(crate) fn emit_tree_sitter(folder: &str) -> Result<(), String> {
    let content = make_grammar_js();
    let folder = Path::new(folder);
    let tree_sitter_project_path = folder.join("tree-sitter-zngur");
    std::fs::create_dir(&tree_sitter_project_path)
            .map_err(|err| format!("{}", err))?;
    let grammar_js_path = tree_sitter_project_path.join(
            "grammar.js"
        );
    std::fs::write(grammar_js_path, content)
            .map_err(|err| format!("{}", err))
}

// fn conditional_item (conditional.rs:560)
//  is invoked twice:
// - lib.rs:3143: conditional_item::<_, CfgConditional, NItems>(item)
// - lib.rs:3333: conditional_item::<_, CfgConditional, NItems>(item)
// Both invocations use the same parameters,
//      and therefore this can be inlined.
//
// pub fn conditional_item(item_parser = item) -> Parser {
//  let if_parser = <CfgConditional as Conditional<NItems>>::if_parser(item.clone());
//  let match_parser = <CfgConditional as Conditional<NItems>>::match_parser(item);
//  choice((if_parser, match_parser))
// }
// 
// // using the conditional.rs:616 :
// //   impl Conditional<..., NItems> for Scrutinee :
// // and the cfg.rs:261 :
// //   impl Matchable for CfgConditional
// impl<Scrutinee = <CfgConditional as MatchableParse>>
//      Conditional<..., NItems> for Scrutinee
//      where <Scrutinee as Matchable>::Pattern = <CfgPattern as MatchPatternParse<'src>> {
//      type Scrutinee = CfgConditional;
//      fn if_parser(item_parser = item) -> Parser {
//          let block = block_for_many(item);
//          let guard = guarded_block::<CfgConditional, NItems>(block);
//          if_stmnt::<CfgConditional, NItems>(guard, block)
//      }
//
//      fn match_parser(item_parser) -> Parser {
//          let block = block_for_many(item);
//          match_stmt::<CfgConditional, NItems>(block, item)
//      }
//  }
//
// // inlining the above:
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = block_for_many(item);
//      let if_parser = {
//          let guard = guarded_block::<CfgConditional, NItems>(block);
//          if_stmt::<CfgConditional, NItems>(guard, block)
//      };
//      let match_parser = {
//          match_stmt::<CfgConditional, NItems>(block, item)
//      };
//      choice((if_parser, match_parser))
// }
// 
//  
// // inlining conditional.rs:377$block_for_many :
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = format!("repeat($.item)");
//      let if_parser = {
//          let guard = guarded_block::<CfgConditional, NItems>(block.clone());
//          if_stmt::<CfgConditional, NItems>(guard, block.clone())
//      };
//      let match_parser = {
//          match_stmt::<CfgConditional, NItems>(block, item)
//      };
//      choice((if_parser, match_parser))
// }
// 
// // guarded_block from conditional.rs:384 :
// pub fn guarded_block<
//      Scrutinee = <CfgConditional as MatchableParse>,
//      Cardinality = NItems
//   >(block: Parser) -> Parser
//          where <CfgConditional as Matchable>::Pattern =
//                  <CfgPattern as MatchPatternParse> {
//      // combined() instance: cfg.rs:349
//      format!(r#"seq(
//          'cfg','!',
//          '(',
//          {CfgScrutinee::parser},
//          optional(seq('=', {CfgPattern::parser})),
//          ')'
//      )"#, CfgScrutinee::parser = CfgScrutinee::parser);
// }
//
// // inlining conditional.rs:384$guarded_block
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = format!("repeat($.item)");
//      let if_parser = {
//          let guard = format!(r#"seq(
//                          'cfg','!','(',
//                          {CfgScrutineeParser},
//                          optional(seq('=', {CfgScrutineeParser})),
//                          ')'
//                      "#,
//                      CfgScrutineeParser = CfgScrutinee::parser
//                  );
//          if_stmt::<CfgConditional, NItems>(guard, block.clone())
//      };
//      let match_parser = {
//          match_stmt::<CfgConditional, NItems>(block, item)
//      };
//      choice((if_parser, match_parser))
// }
//
// // CfgScrutinee::parser @ cfg.rs:311 :
// fn parser() -> Parser {
//     format!(r#"
//         seq( // 'at least 1, at most 2'
//             choice($.ident, $.plain_string),
//             optional(seq(
//                 '.', // separated by Dot
//                 choice($.ident, $.plain_string)
//             ))
//         )
//     "#)
// }
//
// // inlining cfg.rs:311$CfgScrutinee::parser
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = format!("repeat($.item)");
//      let if_parser = {
//          let guard = format!(r#"seq(
//                          'cfg','!','(',
//                          {CfgScrutineeParser},
//                          optional(seq('=', {CfgScrutineeParser})),
//                          ')'
//                      "#,
//                      CfgScrutineeParser = r#"seq(
//                          choice($.ident, $.plain_string),
//                          optional(seq(
//                              '.', choice($.ident, $.plain_string)
//                          ))
//                      )"#
//                  );
//          if_stmnt::<CfgConditional, NItems>(guard, block.clone())
//      };
//      let match_parser = {
//          match_stmt::<CfgConditional, NItems>(block, item)
//      };
//      choice((if_parser, match_parser))
// }
//
// // conditional.rs:466 :
// pub fn if_stmnt<
//              Scrutinee = <CfgConditional as MatchableParse>,
//              Cardinality = <NItems as ConditionBodyCardinality>
//          >(guard: Parser = guard, fallback: Parser = block) -> Parser {
//      format!(r#"seq(
//          '#','if', {guard},
//          repeated(seq('#','else','if', {guard})),
//          optional(seq(
//              '#','else',
//              '{{', {fallback}, '}}'
//          ))
//      )"#, fallback = block)
// }
//
// // inlining conditional.rs:466$if_stmnt
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = format!("repeat($.item)");
//      let if_parser = {
//          let guard = format!(r#"seq(
//                          'cfg','!','(',
//                          {CfgScrutineeParser},
//                          optional(seq('=', {CfgScrutineeParser})),
//                          ')'
//                      "#,
//                      CfgScrutineeParser = r#"seq(
//                          choice($.ident, $.plain_string),
//                          optional(seq(
//                              '.', choice($.ident, $.plain_string)
//                          ))
//                      )"#
//                  );
//          format!(r#"seq(
//              '#', 'if', {guard},
//              repeated(seq('#', 'else', 'if', {guard})),
//              optional(seq('#', 'else', '{{', {block}, '}}')),
//          )"#, guard = &guard, block = &block)
//      };
//      let match_parser = {
//          match_stmt::<CfgConditional, NItems>(block, item)
//      };
//      choice((if_parser, match_parser))
// }
//
// // conditional.rs:534$match_stmt :
// fn match_stmt<
//      Scrutinee = <CfgConditional as MatchableParse>,
//      Cardinality = <NItems as ConditionBodyCardinality>
//  >(block: Parser, item_parser: Parser = item) -> Parser
//          where <CfgConditional as Matchable>::Pattern =
//                  <CfgPattern as MatchPatternParse> {
//      let arm = match_arm::<CfgConditional, NItems>(block, item);
//
//      format!(r#"seq(
//          '#','match',
//          {<CfgConditional as MatchableParse>::parser},
//          '{{', repeat({arm}, optional(',')), '}}'
//      )"#)
// }
//
// // conditional.rs:503$match_arm :
// fn match_arm<
//      Scrutinee = <CfgConditional as MatchableParse>,
//      Cardinality = <NItems as ConditionBodyCardinality>
//  >(block: Parser = block, item_parser: Parser = item) -> Parser
//          where <CfgConditional as Matchable>::Pattern =
//                  <CfgPattern as MatchPatternParse> {
//      let arm_choices = format!(r#"choices(
//              seq('{{', {block}, '}}'), // block
//              {item_parser}, // single
//              seq('(', ')') // empty block
//          )"#);
//      format!(r#"seq(
//              {CfgPatternParser},
//              "=>", {arm_choices}
//          )"#,
//          CfgPatternParser =
//              <CfgPattern as MatchPatternParse>::parser
//      )
// }
//
// // inlining match_arm into match_stmt:
// fn match_stmt<
//      Scrutinee = <CfgConditional as MatchableParse>,
//      Cardinality = <NItems as ConditionBodyCardinality>
//  >(block: Parser, item_parser: Parser = item) -> Parser
//          where <CfgConditional as Matchable>::Pattern =
//                  <CfgPattern as MatchPatternParse> {
//      let arm_choices = format!(r#"choices(
//              seq('{{', {block}, '}}'), // block
//              {item_parser}, // single
//              seq('(', ')') // empty block
//          )"#);
//      let arm = format!(r#"seq(
//              {CfgPatternParser},
//              '=>', {arm_choices}
//          )"#,
//          CfgPatternParser =
//              <CfgPattern as MatchPatternParse>::parser
//      );
//
//      format!(r#"seq(
//          '#','match',
//          {CfgConditionalParser},
//          '{{', repeat({arm}, optional(',')), '}}'
//      )"#, CfgConditionalParser =
//          <CfgConditional as MatchableParse>::parser)
// }
// 
// // inlining match_stmt into conditional_item:
// pub fn conditional_item(item_parser = item) -> Parser {
//      let block = format!("repeat($.item)");
//      let if_parser = {
//          let guard = format!(r#"seq(
//                          'cfg','!','(',
//                          {CfgScrutineeParser},
//                          optional(seq('=', {CfgScrutineeParser})),
//                          ')'
//                      "#,
//                      CfgScrutineeParser = r#"seq(
//                          choice($.ident, $.plain_string),
//                          optional(seq(
//                              '.', choice($.ident, $.plain_string)
//                          ))
//                      )"#
//                  );
//          format!(r#"seq(
//              '#', 'if', {guard},
//              repeated(seq('#', 'else', 'if', {guard})),
//              optional(seq('#', 'else', '{{', {block}, '}}')),
//          )"#, guard = &guard, block = &block)
//      };
//      let match_parser = {
//          let arm_choices = format!(r#"choices(
//                  seq('{{', {block}, '}}'), // block
//                  {item_parser}, // single
//                  seq('(', ')') // empty block
//              )"#, block=&block, item_parser=&item);
//          let arm = format!(r#"seq(
//                  {CfgPatternParser},
//                  '=>', {arm_choices}
//              )"#,
//              CfgPatternParser =
//                  <CfgPattern as MatchPatternParse>::parser
//          );
//
//          format!(
//              r#"seq(
//                  '#','match',
//                  {CfgConditionalParser},
//                  '{{', repeat({arm}, optional(',')), '}}'
//              )"#,
//              CfgConditionalParser =
//                  <CfgConditional as MatchableParse>::parser
//          )
//      };
//      choice((if_parser, match_parser))
// }
//
// <CfgPattern as MatchPatternParse>::parser @ cfg.rs:201 :
fn cfg_pattern_parser(parser_map: &mut Parsers) {
     parser_map.insert("match_pattern_parser_single", format!(r#"
         $ => {{
             let atom = choice( // atom ... .or()
                 choice( // atom.choice(...)
                     choice($.plain_string, $.number), // literals
                     '_',
                     'Some',
                     'None'
                 ),
                 // these're hard because of the fold
                 seq('(', $.match_pattern_parser_single, ')')
             );
             // this's imitating precedence as rules,
             //      if we want to use tree-sitter's precedence later
             let not_pat = seq(repeat('!'), atom);
             let and_pat = seq(
                         not_pat,
                         repeat(seq('&', not_pat))
                     );
             let or_pat = seq(
                 and_pat,
                 repeat(seq('|', and_pat))
             );
             or_pat
         }}
     "#));

     parser_map.insert("match_pattern_parser", format!(r#"
         $ => choice(
             seq(
                 '(',
                 $.match_pattern_parser_single,
                 repeat(seq(',', $.match_pattern_parser_single)),
                 ')'
             ),
             $.match_pattern_parser_single
         )
     "#));
}
//
// <CfgConditional as MatchableParse>::parser @ cfg.rs:331 :
fn cfg_conditional_parser(parser_map: &mut Parsers) {
    let directive = format!(
        r#"$ => seq(
            'cfg', '!', '(', $.cfg_scrutinee, ')'
        )"#
    );
    parser_map.insert(
        "cfg_conditional_parser_directive",
        directive
    );

    let cfg_conditional_parser = format!(r#"$ => choice(
        {directive},
        seq(
            '(',
            {directive},
            repeat(',', {directive}), optional(','),
            ')'
        )
    )"#, directive = "$.cfg_conditional_parser_directive");
    parser_map.insert(
        "cfg_conditional_parser",
         cfg_conditional_parser
    );
}
fn conditional_item() -> Parsers {
    let mut parser_map = HashMap::<&'static str, String>::new();

    //let item = "$.item";
    let block = "repeat($.item)";

    let if_parser = {
        let cfg_scrutinee_parser = r#"$ => seq(
            choice($.ident, $.plain_string),
            optional(seq(
                '.', choice($.ident, $.plain_string)
            ))
        )"#;
        parser_map.insert(
            "cfg_scrutinee",
            cfg_scrutinee_parser.into()
        );

        let guard = format!(
                r#"$ => seq(
                    'cfg','!','(',
                    $.cfg_scrutinee,
                    optional(seq('=', $.cfg_scrutinee)),
                    ')'
                "#
            );
        parser_map.insert("conditional_if_parser_guard", guard);

        format!(
            r#"$ => seq(
                '#', 'if', {guard},
                repeated(seq('#', 'else', 'if', {guard})),
                optional(seq('#', 'else', '{{', {block}, '}}')),
            )"#,
            guard = "$.conditional_if_parser_guard",
            block = &block
        )
    };
    parser_map.insert("conditional_if", if_parser);

    let match_parser = {
        let arm_choices = format!(
            r#"choices(
                seq('{{', {block}, '}}'), // block
                $.item, // single
                seq('(', ')') // empty block
            )"#, block=&block
        );

        cfg_pattern_parser(&mut parser_map);
        let arm = format!(
            r#"seq(
                $.cfg_pattern_parser,
                '=>', {arm_choices}
            )"#
        );

        cfg_conditional_parser(&mut parser_map);
        format!(
            r#"
                $ => seq(
                    '#','match',
                    $.cfg_conditional_parser,
                    '{{', repeat(seq({arm}, optional(','))), '}}'
                )
            "#
        )
    }; // match_parser
    parser_map.insert("conditional_match", match_parser);

    parser_map.insert(
        "conditional_item",
        "$ => choice($.conditional_if, $.match_parser)".into()
    );

    parser_map
}
