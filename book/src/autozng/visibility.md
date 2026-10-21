# Default visibility

By default, Autozng exports the **public interface** of the crate:
everything a user of the crate can reach is written to the `.zng` file,
and everything that is only visible inside the crate is left out.

These are exported by default:

- `pub` types: structs, enums and unions.
- The `pub` fields of those types.
- The `pub` methods in their `impl` blocks.
- `pub` free functions.
- Items the crate re-exports with `pub use`, under the path they are re-exported at.

These are not exported by default:

- Items that are not visible outside the crate (`pub(crate)`, `pub(super)`, or private).
- Traits. Even a `pub` trait is only exported when you tag it with
  [`emit`](./attributes.md#emit-and-skip).
- Private fields and methods.

Autozng also declares the types that an exported item mentions,
so the interface is complete.
A type used in an exported field or signature gets its own `type` block,
even when the type itself is not public.

## A simple example

Given this crate:

```rust
mod inner {
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
}

pub use inner::Point;

pub fn origin() -> Point {
    Point { x: 0, y: 0 }
}
```

Autozng writes:

```zng
type crate::Point {
    #layout(size = 8, align = 4);

    field x (offset = 0, type = i32);
    field y (offset = 4, type = i32);
}

fn crate::origin() -> crate::Point;
```

The type is named `crate::Point`, not `crate::inner::Point`:
Autozng uses the path the item is re-exported at,
because that is the name users of the crate see.

If you want to export something that is not public, or a trait,
ask for it explicitly with [`emit`](./attributes.md#emit-and-skip).
