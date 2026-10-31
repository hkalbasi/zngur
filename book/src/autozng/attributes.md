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

- `emit_traits = true` — treat the enumerated `dyn Trait<...>` items as concrete trait
  instantiations instead of as types. The default is `false`.

A generic trait is only representable once its type arguments are known,
so name the instantiations you need:

```rust
pub trait Producer<T> {
    fn produce(&self) -> T;
}

#[autozng::emit_items(emit_traits = true)]
const _: [(&dyn Producer<u32>, &dyn Producer<String>); 0] = [];
```

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
