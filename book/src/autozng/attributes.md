# Attributes

Autozng reads `#[autozng::...]` attributes to control what it exports.

## Enabling the attributes

The attributes only exist while Autozng runs.
Declare the tool at the top of the crate's root module:

```rust
#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
```

The `autozng` configuration flag is set by the driver, never by a normal build,
so a plain `cargo build` ignores those lines.
Write the attributes themselves the same way, so the crate keeps compiling without Autozng:

```rust
#[cfg_attr(autozng, autozng::emit)]
pub struct Example {
    pub first: String,
}
```

The examples below write the attributes as `#[autozng::...]` for readability.
Add the `cfg_attr` wrapper in real code.

## `emit` and `skip`

`emit` forces an item to be exported; `skip` leaves it out.

Without an attribute, an item is exported when it is part of the public interface
(see [Default visibility](./visibility.md)).
`emit` also exports an item that is not public,
as long as the crate can name it from the crate root,
and `skip` removes an item that would otherwise be exported.

`emit` and `skip` take either no arguments or a list:

- no arguments — the item itself: `#[autozng::emit]`, `#[autozng::skip]`.
- a list of names — the named children, such as fields and methods:
  `#[autozng::emit(third, forth)]`.
- `self` inside a list — the item itself: `#[autozng::emit(self, extra)]`.

They can be placed on types, fields, methods, free functions, traits, `impl` blocks,
and `let` bindings.

A few cases are worth calling out:

- A trait must be tagged with `emit` to be exported at all.
- A generic trait cannot be exported as a `trait` block; name each concrete
  instantiation with `emit_items(emit_traits = true)` instead.
- Union fields are not exported. Autozng skips them silently, and `emit` on one is an error.
- `skip` on an `impl` block leaves out every method in it.
- `emit` on an item the crate cannot name from the crate root has no effect,
  and Autozng reports it.

```rust
pub struct Example {
    pub first: String,
    #[autozng::skip]
    pub second: i32,
    #[autozng::emit]
    third: u32,
}
```

## `emit_items`

Exports the concrete types and functions listed inside a `const`.

A generic type is only usable in the interface once its type arguments are known,
so `Wrapper<T>` cannot be exported on its own.
`emit_items` lets you name the instantiations you need.
Put it on a `const` and list the types in the const's type:

```rust
pub struct Wrapper<T>(pub T);

#[autozng::emit_items]
const _: &[(Wrapper<u32>, Wrapper<String>)] = &[];
```

To export a specific function instantiation, list it with a `let` binding in the const's body:

```rust
impl Wrapper<u32> {
    pub fn get<T>(&self, other: T) -> T {
        other
    }

    #[autozng::emit_items]
    const _: () = {
        let _ = Self::get::<u8>;
        #[autozng::rename("get_string")]
        let _ = Self::get::<String>;
    };
}
```

A `let` binding may carry its own [`rename`](#rename-and-rename_all),
which applies to that instantiation only.

`emit_items` takes one optional argument:

- `emit_traits = true` — treat every `dyn Trait<...>` in the enumerated items as a concrete
  trait instantiation instead of as a trait-object type. The default is `false`.

A generic trait is only representable once its type arguments are known,
so name the instantiations you need.
Put them in the const's body, one per line:

```rust
pub trait Producer<T> {
    fn produce(&self) -> T;
}

#[autozng::emit_items(emit_traits = true)]
const _: () = {
    let _: &dyn Producer<u32>;
    let _: &dyn Producer<String>;
};
```

The `dyn Trait` may sit inside any type the binding names (`Option<&dyn Trait>`,
`Box<dyn Trait>`, ...): every trait object in it names its principal trait, and auto traits such
as `Send` are ignored.
The const's type works too: `const _: [&dyn Producer<u32>; 0] = [];`.

With `emit_traits = true` each instantiation becomes a `trait` block, which C++ types can
inherit from (see [Creating trait objects from C++ types](../call_cpp_from_rust/trait_object.md)):

```zng
trait crate::Producer<u32> {
    fn produce(&self) -> u32;
}
```

A trait must be dyn compatible to be exported this way: a trait with generic methods, an
associated constant, or a static method is rejected. A static method with `where Self: Sized`
is dyn compatible, but a C++ type has no way to implement it, so the trait block is rejected.
A `pub use` of another crate's items never exports its traits; select a trait explicitly.

Leave `emit_traits` off to export the trait object *type* instead: the same binding produces
`type dyn crate::Producer<u32>`, which is what you need to call methods on a trait object
created in C++.

## `emit_on_implementers`

Exports a trait's methods on every type that implements the trait.

Normally you list a type's methods one by one, or enumerate the ones you need with
[`emit_items`](#emit_items).
With `emit_on_implementers` you tag the trait instead,
and its methods are added to each type that implements it:

```rust
#[autozng::emit]
#[autozng::emit_on_implementers]
pub trait Describe {
    fn describe(&self) -> String;
}
```

Every implementing type now carries the method:

```zng
type crate::Example {
    // ...
    fn describe(&self) -> ::std::string::String use crate::Describe;
}
```

The attribute can also be placed on a trait `impl` block,
in which case the methods are exported only on the types that block covers.

The trait must be public (or `pub(crate)`).
Tag it with [`emit`](#emit-and-skip) as well if you also want a `trait` block of its own.

## `rename` and `rename_all`

Changes the name a method is exported under.

`rename` changes the item's own name; `rename_all` changes the names of its children
(fields and methods).
Both accept a list of actions, applied in order:

- `"new_name"` or `name = "new_name"` — replace the name.
- `case = "Case"` — convert the name to a case.
- `prefix = "pre"` — add a prefix.
- `suffix = "post"` — add a suffix.

`rename_all` does not accept a replacement name, only `case`, `prefix` and `suffix`.

The available case names are `snake_case`, `CONSTANT` (or `UPPER_SNAKE`),
`Ada` (or `Capital_Snake`), `flat`, `UPPERFLAT`, `Pascal` (or `UpperCamel`), and `camelCase`.

```rust
impl Example {
    #[autozng::rename(case = "Pascal")]
    pub fn a_long_fn_that_will_be_renamed(&self) {}
}
```

The method is exported with the new name next to the original one:

```zng
fn a_long_fn_that_will_be_renamed(&self) as ALongFnThatWillBeRenamed;
```

The Rust name is kept because Zngur needs it to call the method,
while the C++ binding uses the exported name
(see [Name mapping](../call_rust_from_cpp/name_mapping.md)).

Currently only methods are renamed;
`rename` on a type, field or free function is accepted but has no effect.

A `rename` on a `let` binding inside an [`emit_items`](#emit_items) const applies only to the
instantiation that the binding names: `foo::<u32>` does not rename `foo::<u8>`, and
`Generic::<u32>::method` does not rename the method on `Generic<String>`. Name each instantiation
you want to rename. A `rename` on the method declaration applies to every instantiation.

`rename(method = "name", ...)` on an ADT or a type alias targets one of its inherent methods
instead of the item itself, and applies to every instantiation:

```rust
#[autozng::rename(method = "get", suffix = "Cpp")]
pub struct Generic<T> {
    inner: T,
}
```

A type that cannot carry the attribute (an external type) can be named with a type alias inside an
`emit_items` const:

```rust
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(method = "reverse", suffix = "Cpp")]
    type OrderingAlias = ::std::cmp::Ordering;
};
```

`method = "..."` is only valid on an ADT or a type alias; a `rename` on a type alias must use it,
because the alias itself is not emitted. Several `rename` attributes on one item are merged, so a
type rename and a method rename can coexist. A specific instantiation rename still wins over a
type-level one.

`rename(trait_path = "::std::convert::From<u32>", ...)` on an ADT or a type alias renames the
methods of that trait when they are emitted on that type (`emit_on_implementers` or an
`emit_items` binding). It applies only to that type, never to other implementers of the trait.
`method = "..."` narrows it to one method; without it every method of the trait is renamed.

The selector is matched exactly against the trait reference as it appears in the `.zng`: the
visible path (`crate::Trait` or `::std::...`) and, when given, the argument list in the IDL form
(`::std::string::String`, `crate::Foo<u32>`). Whitespace is ignored; relative paths are not
resolved. Without arguments the selector matches every instantiation of the trait on the type;
with arguments it matches only that rendered instantiation. A selector that matches no emitted
trait method warns. `rename_all` does not accept `trait_path`.

## `layout`

Overrides the layout policy of a type.

Autozng normally writes the type's real size and alignment:

```zng
type crate::Point {
    #layout(size = 8, align = 4);
}
```

`#[autozng::layout(...)]` replaces that with another
[layout policy](../call_rust_from_cpp/layout_policy.md).
Exactly one of the following is allowed per type:

- `stack(size = N, align = N)` — the values must match the real size and alignment.
- `conservative(size = N, align = N)` — the values must be at least the real size and alignment.
- `heap_allocated` — the type needs no size or alignment.
- `only_by_ref` — the type can only be used by reference.

```rust
#[autozng::layout(heap_allocated)]
pub struct Big {
    pub data: [u8; 4096],
}
```

A `stack` or `conservative` value that is wrong is a compile error,
so Autozng checks the numbers for you.

## `walk_filter`

Selects which of a type's fields, variants and methods the recursive interface walk exports.

The attribute is accepted on the crate root (the default for every walk) and on emit-able items
(types, fields, methods, traits, free functions and `emit_items` consts).
A nested filter replaces the inherited one for its whole subtree.
It takes an ordered list of roots:

```rust
#![autozng::walk_filter(exclude(TraitMethods))]
#[autozng::walk_filter(include(Fields, InherentMethods))]
#[autozng::walk_filter(include(Named("value"), Named("get")))]
```

Each matching root sets the verdict (`all` / `include` allow, `none` / `exclude` deny), and the
last matching root wins.

The item rules are `Fields`, `Variants`, `InherentMethods`, `TraitMethods`, `Named("...")`,
`Receiver(Value | Ref | RefMut | Static)` and `Visibility(Public | PublicInternal | Private)`.

The owner rules match every item of a type:

- `Crate("std")` — the type's **defining** crate, not its `.zng` path. A re-exported external
  type (for example `pub use png::*`) still matches `Crate("png")` even though its path is
  `crate::...`.
- `PathPrefix("crate::module")` — a segment-wise prefix of the type's **public** `.zng` path.
  `crate` is the analyzed crate, and a re-export keeps its re-export path. A leading `::` is
  ignored.
- `External` — the type is defined outside the analyzed crate.

For example, with [`implicitly_emit_external_interfaces`](#policy), keep the local interface and
the re-exported crate's own types while making `std`/`core`/`alloc` layout-only:

```rust
#![autozng::walk_filter(exclude(Crate("std"), Crate("core"), Crate("alloc")))]
```

`exclude(External)` drops every external interface; `PathPrefix` narrows the walk by module.
Owner rules are useful with generic types, where the path is the base type's path without its
arguments.

Slices and `str` have a default filter, because their full interfaces pull in large amounts of
iterator API that a C++ caller rarely needs:

- `[T]` — `len`, `is_empty`, `as_ptr`, `as_mut_ptr`, `contains`, `to_vec`.
- `str` — `len`, `is_empty`, `from_utf8`, `as_bytes`, `as_bytes_mut`.

An explicit `walk_filter` replaces that list, while a bare `emit` or `emit_items` keeps it.
This applies to types enumerated by `emit_items` too:

```rust
#[autozng::emit_items]
#[autozng::walk_filter(include(Named("chunks")))]
const _: [&[u8]; 0] = [];
```

The slice above exports only `chunks` instead of the six defaults.

## `policy`

Sets a policy for the whole crate. It is a crate-level attribute:

```rust
#![autozng::policy(implicitly_emit_external_interfaces)]
```

Available policies:

- `implicitly_emit_external_interfaces` — also export the fields and methods of the external
  types (such as `String` and `Vec`) that the interface reaches,
  instead of only declaring the types themselves.

This can make the generated file much larger, so it is opt-in.

Unstable (`#[unstable]`) items are skipped silently. The generated bridge is currently built
with the stable toolchain; nightly bridges are not supported yet because the driver cannot know
which unstable features the target crate's nightly exposes.
