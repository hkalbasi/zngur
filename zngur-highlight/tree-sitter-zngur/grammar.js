
/**
 * @file A Parser for the Zngur C++/Rust bindings language
 * @author cplir-c 31234188+cplir-c@users.noreply.github.com
 * @license Dual Apache 2.0 and MIT license
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

export default grammar({
    name: "zngur",
    extras: $ => [ /\s/, $.comment ],
    reserved: {
        keywords: $ => ['as', 'async', 'dyn', 'mod', 'type', 'trait', 'crate', 'fn', 'mut', 'const', 'use', 'for', 'extern', 'impl', 'import', 'merge', 'if', 'else', 'match', 'safe', 'unsafe'],
    },
    word: $ => $.identifier,
    //externals: $ => [$.raw_string],
    rules: {
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
            $.conditional_item // 1
        ),
        comment: $ => seq("//", /[^\n]*/),
        identifier:
            $ => new RustRegex('\\p{XID_Start}\\p{XID_Continue}*'),
        ident: $ => alias($.identifier, $.ident),
        number: $ => /0|[1-9][0-9]+/,

        // ... cancel this, we're using Rust-side text templates
        cfg_conditional_parser_directive: $ => seq(
            'cfg', '!', '(', $.cfg_scrutinee, ')'
        ), cfg_match_pattern_parser_single: 
         $ => {
             let atom = choice( // atom ... .or()
                 choice( // atom.choice(...)
                     choice($.plain_string, $.number), // literals
                     '_',
                     'Some',
                     'None'
                 ),
                 // these're hard because of the fold
                 seq('(', $.cfg_match_pattern_parser_single, ')')
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
             return or_pat;
         }
     , cfg_match_pattern_parser: 
         $ => choice(
             seq(
                 '(',
                 $.cfg_match_pattern_parser_single,
                 repeat(seq(',', $.cfg_match_pattern_parser_single)),
                 ')'
             ),
             $.cfg_match_pattern_parser_single
         )
     , conditional_if_parser_guard: $ => seq(
                    'cfg','!','(',
                    $.cfg_scrutinee,
                    optional(seq('=', $.cfg_scrutinee)),
                    ')'
                ), conditional_if: $ => seq(
                '#', 'if', $.conditional_if_parser_guard,
                repeat(seq('#', 'else', 'if', $.conditional_if_parser_guard)),
                optional(seq('#', 'else', '{', repeat($.item), '}'))
            ), cfg_conditional_parser: $ => choice(
        $.cfg_conditional_parser_directive,
        seq(
            '(',
            $.cfg_conditional_parser_directive,
            repeat(seq(',', $.cfg_conditional_parser_directive)), optional(','),
            ')'
        )
    ), conditional_item: $ => choice($.conditional_if, $.conditional_match), cfg_scrutinee: $ => seq(
            choice($.ident, $.plain_string),
            optional(seq(
                '.', choice($.ident, $.plain_string)
            ))
        ), conditional_match: 
                $ => seq(
                    '#','match',
                    $.cfg_conditional_parser,
                    '{', repeat(seq(seq(
                $.cfg_match_pattern_parser,
                '=>', choice(
                seq('{', repeat($.item), '}'), // block
                $.item, // single
                seq('(', ')') // empty block
            )
            ), optional(','))), '}'
                )
            ,

        unstable_feature: $ => seq(
            '#', 'unstable',
            // ident can be 'cfg_match', 'cfg_if', or 'template_types'
            '(', $.ident, ')'
        ),

        module_import_item: $ => seq(
            'import', $.plain_string, ';'
        ),

        import_item: $ => seq(
            'merge', $.plain_string, ';' // 1
        ),

        additional_include_item: $ => seq(
            '#',
            choice(
                seq(
                    'cpp_additional_includes',
                    $.rawable_string // 1
                ),
                'convert_panic_to_exception'
            )
        ),

        fn_item: $ => seq(
            optional(choice('safe', 'unsafe')),
            $.function_signature, ';' // 1
        ),

        extern_cpp_item: $ => {
            const safety = choice('safe', 'unsafe');
            const function_ = seq(
                safety, $.method_signature, ';' // 1, 1
            );
            // impl RustTrait for RustType { safety method; ... }
            const impl_block = seq(
                'impl',
                optional(seq(
                    $.rust_trait, // 1
                    'for',
                )),
                $.rust_type, // 1
                '{',
                repeat(seq(
                    safety, $.method_signature, ';' // 1, 1
                )),
                '}',
            );
            return seq(
                'extern',
                '"C++"',
                '{',
                repeat(choice(function_, impl_block)),
                '}'
            );
        },

        trait_item: $ => seq(
            'trait',
            $.rust_trait, // 1
            '{', repeat(seq($.method, ';')), '}' // TODO
        ),

        type_item: $ => seq(
            'type',
            optional(seq(
                '<',
                $.ident, repeat(seq(',', $.ident)), optional(','),
                '>'
            )),
            $.rust_type, // 1
            '{', repeat($.inner_type_item), '}' // 1
        ),

        module: $ => seq(
            'mod',
            $.path, // 1
            $.braced_items // 1
        ),
        braced_items: $ => seq(
            // 1, 1, 1
            '{', repeat($.item), '}'
        ),
        // lib.rs:3367
        path: $ => {
            const start = optional(seq(
                choice(
                    '::',
                    seq(
                        'crate',
                        '::'
                    )
                )
            ));

            return prec.left(seq(
                start, // 1
                $.ident, // 1
                repeat(seq('::', $.ident)) // 1, 1
            ));
        },
        // lib.rs:606
        // e.g. ? `use ::std::boxed::Box as Boxed;`
        alias: $ => seq(
            'use',
            $.path, // 1
            'as',
            $.ident, // 1
            ';'
        ),

        // lib.rs:2865
        function_signature: $ => {
            const function_path = $.path; // 1

            return seq(
                optional('async'), 'fn',
                function_path, // 1
                optional(seq(
                    '<',
                    optional(seq(
                        $.rust_type, // 1
                        repeat(seq(',', $.rust_type)) // 1
                    )),
                    '>'
                )),
                $.fn_args // 1
            );
        },

        // lib.rs:2842
        // Just like the function_signature,
        //      but with an identifier name instead of a path
        method_signature: $ => seq(
            optional('async'),
            'fn',
            $.ident,
            optional(seq(
                '<',
                optional(seq(
                    $.rust_type, // 1
                    repeat(seq(',', $.rust_type)) // 1
                )),
                '>'
            )),
            $.fn_args // 1
        ),

        // lib.rs:2811
        rust_trait: $ => {
            const fn_trait = seq(
                $.ident, // 1
                $.fn_args // 1
            );

            return choice(
                fn_trait,
                $.rust_path_and_generics // 1
            );
        },

        // lib.rs:2633
        rust_type_trait_bound: $ => seq(
            '+', $.path
        ),
        rust_type: $ => {
            const adt = $.rust_path_and_generics; // 1

            const dyn_trait = seq(
                choice(
                    'dyn', 'impl'
                ),
                $.rust_trait,
                repeat($.rust_type_trait_bound)
            );

            const boxed = seq(
                'Box',
                $.rust_generics
            );

            const unit = seq('(', ')');
            const tuple = seq(
                '(',
                $.rust_type,
                repeat(seq(',', $.rust_type)),
                optional(','),
                ')'
            );
            const slice = seq('[', $.rust_type, ']');
            const reference = seq(
                '&', optional('mut'), $.rust_type
            );
            const raw_ptr = seq(
                '*', choice('mut', 'const'), $.rust_type
            );

            return choice(
                $.rust_scalar,
                boxed,
                unit,
                tuple,
                slice,
                adt,
                reference,
                raw_ptr,
                dyn_trait
            );
        }, // rust_type
        rust_scalar: $ => choice(
            'bool', 'str', 'char', 'usize',
            'u8', 'u16', 'u32', 'u64', 'u128',
            'i8', 'i16', 'i32', 'i64', 'i128',
            'f32', 'f64'
        ),

        // lib.rs:52
        inner_type_property_item: $ => seq(
            $.ident, '=', $.number
        ),
        inner_type_trait_item: $ => choice(
            'Debug',
            'Copy',
            seq('?', 'Sized')
        ),
        inner_type_constructor_arg: $ => seq(
            choice(
                seq(
                    '(',
                    optional(seq(
                        $.rust_type,
                        repeat(seq(',', $.rust_type))
                    )),
                    ')'
                ),
                $.ident
            ),
            ':',
            $.rust_type
        ),
        inner_type_field: $ => seq(
            'field',
            choice($.ident, $.number),
            '(',
            repeat(seq(
                'offset', '=', choice($.number, 'auto'),
                ',', 'type', '=', $.rust_type
            )),
            ')'
        ),
        inner_type_inner_item: $ => {
            const layout = choice(
                seq(
                    '#', 'layout',
                    '(',
                    optional(seq(
                        $.inner_type_property_item,
                        repeat(seq(',', $.inner_type_property_item))
                    )),
                    ')'
                ), seq(
                    '#', 'layout_conservative',
                    '(',
                    optional(seq(
                        $.inner_type_property_item,
                        repeat(seq(',', $.inner_type_property_item))
                    )),
                    ')'
                ),
                seq('#', 'only_by_ref'),
                seq('#', 'heap_allocated')
            );

            const traits = seq(
                'wellknown_traits',
                '(',
                optional(seq(
                    $.inner_type_trait_item,
                    repeat(seq(',', $.inner_type_trait_item))
                )),
                ')'
            );
            
            const non_exhaustive = 'non_exhaustive';

            const constructor_args = optional(seq(
                '{',
                optional(seq(
                    $.inner_type_constructor_arg,
                    repeat(seq(',', $.inner_type_constructor_arg))
                )),
                '}'
            ));

            const constructor = seq(
                'constructor',
                constructor_args
            );

            const cpp_value = seq(
                '#', 'cpp_value',
                $.plain_string,
                $.plain_string
            );

            const cpp_heap_allocated = seq(
                '#', 'cpp_heap_allocated',
                $.plain_string
            );

            const cpp_ref = seq(
                '#', 'cpp_ref',
                $.plain_string
            );

            const cpp_stack_owned = seq(
                '#', 'cpp_stack_owned',
                $.plain_string,
                optional(seq(
                    '(',
                    optional(seq(
                        $.inner_type_property_item,
                        repeat(seq(',', $.inner_type_property_item))
                    )),
                    ')'
                ))
            );

            const inner_item = choice(
                layout,
                traits,
                non_exhaustive,
                constructor,
                $.inner_type_field,
                cpp_value,
                cpp_heap_allocated,
                cpp_ref,
                cpp_stack_owned,
                seq(
                    $.method, // TODO
                    optional(seq('use', $.path)),
                    optional(seq('deref', $.rust_type)),
                    optional(seq('as', $.ident))
                )
            );
            return inner_item;
        },

        inner_type_item: $ => {
            const match_stmt = $.inner_type_conditional_item;

            const variant = seq(
                'variant',
                $.ident,
                '{',
                repeat(seq(
                    choice(
                        'non_exhaustive',
                        $.inner_type_field
                    ),
                    ';'
                )),
                '}'
            );

            return choice(
                match_stmt,
                variant,
                seq($.inner_type_inner_item, ';')
            );
        }, // inner_type_item

        inner_type_cfg_scrutinee: $ => seq(
            choice($.ident, $.plain_string),
            optional(seq(
                '.', choice($.ident, $.plain_string)
            ))
        ), inner_type_cfg_conditional_parser: $ => choice(
        $.cfg_conditional_parser_directive,
        seq(
            '(',
            $.cfg_conditional_parser_directive,
            repeat(seq(',', $.cfg_conditional_parser_directive)), optional(','),
            ')'
        )
    ), inner_type_cfg_match_pattern_parser: 
         $ => choice(
             seq(
                 '(',
                 $.cfg_match_pattern_parser_single,
                 repeat(seq(',', $.cfg_match_pattern_parser_single)),
                 ')'
             ),
             $.cfg_match_pattern_parser_single
         )
     , inner_type_cfg_conditional_parser_directive: $ => seq(
            'cfg', '!', '(', $.cfg_scrutinee, ')'
        ), inner_type_cfg_match_pattern_parser_single: 
         $ => {
             let atom = choice( // atom ... .or()
                 choice( // atom.choice(...)
                     choice($.plain_string, $.number), // literals
                     '_',
                     'Some',
                     'None'
                 ),
                 // these're hard because of the fold
                 seq('(', $.cfg_match_pattern_parser_single, ')')
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
             return or_pat;
         }
     , inner_type_conditional_match: 
                $ => seq(
                    '#','match',
                    $.cfg_conditional_parser,
                    '{', repeat(seq(seq(
                $.cfg_match_pattern_parser,
                '=>', choice(
                seq('{', repeat($.inner_type_inner_item), '}'), // block
                $.inner_type_inner_item, // single
                seq('(', ')') // empty block
            )
            ), optional(','))), '}'
                )
            , inner_type_conditional_item: $ => choice($.conditional_if, $.conditional_match), inner_type_conditional_if_parser_guard: $ => seq(
                    'cfg','!','(',
                    $.cfg_scrutinee,
                    optional(seq('=', $.cfg_scrutinee)),
                    ')'
                ), inner_type_conditional_if: $ => seq(
                '#', 'if', $.conditional_if_parser_guard,
                repeat(seq('#', 'else', 'if', $.conditional_if_parser_guard)),
                optional(seq('#', 'else', '{', repeat($.inner_type_inner_item), '}'))
            ),

        // lib.rs:2784
        fn_args: $ => seq(
            '(',
            optional(seq(
                $.rust_type,
                repeat(seq(
                    ',', $.rust_type
                )),
                optional(',')
            )),
            ')',
            optional(seq('->', $.rust_type))
        ),

        rust_path_and_generics: $ => {
            const generics = $.rust_generics; // 1

            return seq(
                $.path,
                optional(generics)
            );
        },

        rust_generics_named_generic: $ => {
            const named_generic = seq(
                $.ident,
                '=',
                $.rust_type
            );
            return named_generic;
        },
        rust_generics: $ => {
            return seq(
                optional('::'),
                '<',
                optional(seq(
                    choice(
                        $.rust_generics_named_generic,
                        $.rust_type
                    ),
                    repeat(seq(
                        ',', choice(
                            $.rust_generics_named_generic,
                            $.rust_type
                        )
                    )),
                    optional(',')
                )),
                '>'
            );
        },

        method: $ => seq(
            optional(choice(
                'safe', 'unsafe'
            )),
            $.method_signature
        ),
        
        plain_string: $ => /"(?:[\\]"|[^"])*"/,
        raw_string: $ => token(choice(seq(
                        new RustRegex('r#{20}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{20}')
                        )
                    ), seq(
                        new RustRegex('r#{19}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{19}')
                        )
                    ), seq(
                        new RustRegex('r#{18}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{18}')
                        )
                    ), seq(
                        new RustRegex('r#{17}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{17}')
                        )
                    ), seq(
                        new RustRegex('r#{16}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{16}')
                        )
                    ), seq(
                        new RustRegex('r#{15}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{15}')
                        )
                    ), seq(
                        new RustRegex('r#{14}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{14}')
                        )
                    ), seq(
                        new RustRegex('r#{13}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{13}')
                        )
                    ), seq(
                        new RustRegex('r#{12}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{12}')
                        )
                    ), seq(
                        new RustRegex('r#{11}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{11}')
                        )
                    ), seq(
                        new RustRegex('r#{10}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{10}')
                        )
                    ), seq(
                        new RustRegex('r#{9}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{9}')
                        )
                    ), seq(
                        new RustRegex('r#{8}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{8}')
                        )
                    ), seq(
                        new RustRegex('r#{7}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{7}')
                        )
                    ), seq(
                        new RustRegex('r#{6}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{6}')
                        )
                    ), seq(
                        new RustRegex('r#{5}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{5}')
                        )
                    ), seq(
                        new RustRegex('r#{4}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{4}')
                        )
                    ), seq(
                        new RustRegex('r#{3}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{3}')
                        )
                    ), seq(
                        new RustRegex('r#{2}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{2}')
                        )
                    ), seq(
                        new RustRegex('r#{1}"'),
                        repeat(token.immediate(/./)),
                        token.immediate(
                            new RustRegex('/"#{1}')
                        )
                    ))),
        rawable_string: $ => choice($.plain_string, $.raw_string)
    }
});
