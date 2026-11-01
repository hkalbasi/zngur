# Merging (originally called Import)

The `merge` directive allows you to include type definitions and other declarations
from other `.zng` files into your main specification.
Types, traits, and modules can appear multiple times across the transitive set of imported files,
and their content is merged together.

## Syntax

```zng
merge "./path/to/file.zng";
```

## Path Resolution

Merge paths are resolved relative to the directory containing the current `.zng` file:

- `merge "./types.zng";`

At this time, absolute paths are not supported.
Importing paths without a leading specifier (e.g. `import "foo/bar.zng";`)
is reserved for a possible future extension.

Above, "the current .zng file" refers to the `.zng` file being parsed,
which is not necessarily the top-level `.zng` file passed to `zngur` on the command line.

## Behavior

When a merge statement is processed:

1. The parser reads and parses the imported file
2. All declarations from the imported file are *merged* into the current specification
3. Imported content becomes available as if it were defined in the importing file
4. Merge processing happens recursively - imported files can themselves contain merge statements

## Merging Algorithm

Zngur's merge algorithm attempts to compute the union of each set of declarations
which share an identity (e.g. every `type crate::Inventory { ... }` across all imported files).
Duplicates are ignored, but contradictions will raise a compiler error.
For example, if two different `type crate::Inventory { ... }` declarations
both specify `wellknown_traits(Debug);`, parsing will succeed.
However, if they specify different layouts, an error will be reported.

## `#convert_panic_to_exception` constraints

`#convert_panic_to_exception` may only appear in a top-level `.zng` file.
This is an application-level decision that should not be determined by dependent libraries.

## Example

**main.zng:**

```zng
merge "./core_types.zng";
merge "./iterators.zng";

// May only appear in the top-level file.
#convert_panic_to_exception

type MyApp {
    #layout(size = 8, align = 8);

    fn run(&self) -> i32;
}
```

**core_types.zng:**

```zng
mod ::std {
    type option::Option<i32> {
        #layout(size = 8, align = 4);
        wellknown_traits(Copy);

        variant None { }
        variant Some {
            field 0 (offset = auto, type = i32);
        }

        fn unwrap(self) -> i32;
    }

    mod vec {
        type Vec<i32> {
            #layout(size = 24, align = 8);
            fn new() -> Vec<i32>;
            fn push(&mut self, i32);
        }
    }
}
```

**iterators.zng:**

```zng
mod ::std {
    mod vec {
        type Vec<i32> {
            fn into_iter(self) -> ::std::vec::IntoIter<i32>;
        }
    }
}
```

In this example, `main.zng` imports type definitions from two separate files,
allowing for better organization of the zngur specification.

Notice that `iterators.zng` is able to "reopen" the `::std::vec::Vec<i32>` specification
and extend it with a single function, `into_iter`.
It does not need to respecify the `#layout` because that is already declared in `core_types.zng`.

# Import

On top of `zngur`s `merge` syntax which enables better file organization,
there's a different flavour of imports that enables zngur bridges to be split
across multiple compilation units (i.e. Rust crate and C++ static libraries).

## Syntax

```zng
import "./path/to/module.zng";
```

## Path Resolution

Path resolution is identical to regular `merge`s.

## Behavior

The direct implication of an `import` is that the generated `.h` will add
an `#include "./path/to/module.zng.h"` (A follow up feature may allow this path
to be user-defined). This enables you to use the generated types from the
external module without regenerating them in this zngur module.

## Example

**main.zng:**

```zng
import "./my_types.zng";

// May only appear in the top-level file per module. Both these files are top level modules.
#convert_panic_to_exception

type MyApp {
    #layout(size = 8, align = 8);

    // Note how I have to assume the crate name of `my_types.zng` is my_types.
    fn run(&self) -> ::my_types::MyOption<i32>;
}
```

**my_types.zng:**

```zng
// May only appear in the top-level file per module. Both these files are top level modules.
#convert_panic_to_exception

mod ::crate {
    type MyOption<i32> {
        #layout(size = 8, align = 4);
        wellknown_traits(Copy);

        variant None { }
        variant Some {
            field 0 (offset = auto, type = i32);
        }

        fn unwrap(self) -> i32;
    }
}
```

## Importing C++ types

`c++::` types don't have a Rust path of their own, so a module that declares them names its crate
with `#crate`, in its top-level file:

**bar.zng** (generated into the crate `bar_bindings`):

```zng
#crate bar_bindings

type c++::ns::Bar {
}
```

Modules that import it can then refer to the type by its `c++::` path, the same way as to their own
`c++::` types:

**main.zng:**

```zng
import "./bar.zng";

mod crate {
    fn use_bar(&c++::ns::Bar);
}
```

zngur reads the imported `.zng` file (if it's at the relative path) to find its `#crate` and the
`c++::` types it declares. Their Rust wrappers are assumed to be at
`::<crate>::generated::<c++ path>` (`::bar_bindings::generated::ns::Bar` above). So the crate must
include its generated code as a public `generated` module at its root:

```rust
// bar_bindings/src/lib.rs
#[rustfmt::skip]
pub mod generated;

// Optional, to also have the types at the root of the crate.
pub use generated::*;
```

`#crate` also sets the crate name of the module itself, so it must match `--crate-name`
(`with_crate_name`) if that's given too. An imported module that declares `c++::` types without a
`#crate` is an error.

## Note: Build System Integration

This feature requires significantly more involvement with the build system
but it's necessary to scale to larger projects that span multiple crates, C++
compilation units, and binaries. To use this feature, your build system should
be able to manage the dependencies between the two modules and guarantee that
it will generate the files required in the right place.

In particular

- Each module should generate its header file in the same directory as the top
  level `.zng` and with the exact same name as the `.zng` with a `.h` extension
  added (e.g. `a/b/c.zng` -> `a/b/c.zng.h`)
- Exactly one Rust static library must be generated and linked to the final
  binary and it should contain all the symbols needed by all the transitive zngur
  bridges
- Every generated header and `zngur.h` header must use the same top level namespace.
  can leave the default "rust" namespace or set your own
- When you import a module to your zngur definition, you have to refer to the types exported
  from that module using their fully qualified name, which means you must know the
  crate name that is utilized in the generated module and this should be constant throughout
  your project (i.e. no aliasing!).

Some build systems may be able to automate this more easily than others but any specific
build system integration is outside the scope of this chapter.
