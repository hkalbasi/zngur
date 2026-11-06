# Opaque C++ types

C++ types are declared with a `c++::` path, which is also their C++ name: `type c++::osmium::Way`
is the C++ type `::osmium::Way`. Multiple declarations sharing a prefix can also be grouped with
`mod c++::osmium { type Way { ... } }`. What Rust can do with such a type follows from what the
`.zng` file says about it:

| Declaration                                | Rust can                                                 |
| ------------------------------------------ | -------------------------------------------------------- |
| no layout                                  | only refer to it (`&T`, `&mut T`)                        |
| a layout, without `#trivially_relocatable` | only refer to it (the layout is checked in C++)          |
| a layout and `#trivially_relocatable`      | hold it by value, like a Rust value                      |
| `#heap_allocated`                          | own a heap-allocated C++ object via pointer + destructor |

> **NOTE**: In C++, the Rust wrapper of a `c++::a::B` type is `rust::cpp::a::B`, apart from the
> wrappers of Rust types (`rust::crate::...`, `rust::std::...`, ...). In Rust, its wrapper is at
> `generated::a::B`.

If a C++ type's name can't be written as a `c++::` path, e.g. because it differs from what you want
to call it in Rust or it's a nested type, give it one with a C++ alias:

```cpp
namespace task { using CppTask = CppTaskForRust; }
```

```zng
type c++::task::CppTask { ... }
```

## Reference-only C++ types

A C++ type without a layout is reference-only:

```zng
type c++::osmium::Way {
}
```

This enables creating `rust::Ref<rust::cpp::osmium::Way>` from a `const osmium::Way&` in C++ and
passing it to the Rust side. Rust side can't do anything meaningful with it, except passing it again
to the C++ side. In the C++ side `rust::Ref<rust::cpp::osmium::Way>` has a `.cpp()` method which
will return the `osmium::Way&` back to you. You can also write `impl` and `impl trait` blocks for
the Rust wrapper `Way` inside C++. See the
[`examples/osmium`](https://github.com/HKalbasi/zngur/blob/main/examples/osmium) for a full working
example.

You can give a reference-only type a layout anyway (`#layout(...)`).
Rust still only refers to it, but the generated C++ code checks the size and alignment.

### Semantics of reference-only types

Reference-only types are implemented as ZSTs that can only be obtained by reference. While the Rust
representation is a ZST, when the reference flows into C++, the reference is pointing to the C++
object.

This can sometimes cause behavior that is safe and sound, but surprising and counterintuitive for
someone that expects them to represent the whole C++ object. If you assume that `RustType` is a
newtype wrapper around a reference to a `CppType` class in C++, instead of a ZST, you will find it
surprising that:

- `std::mem::sizeof::<RustType>()` is 0, not the size of `CppType`
- `std::mem::alignof::<RustType>()` is 1, not the align of `CppType`
- `std::mem::swap::<RustType>(a, b)` only swaps the first zero bytes of those, i.e. does nothing.

Those problem might be solved by the `extern type` language feature.

## Trivially Relocatable C++ Types

If we want to have C++ objects directly within the Rust stack, then things become a bit more
complicated. C++'s move semantics are not compatible with Rust's move semantics.

### C++ Move Semantics

For better or for worse, C++ has very complex machinery to enable efficient movement of data in
memory. In the general case, it uses
[`rvalue` references](https://en.cppreference.com/cpp/language/value_category) in order to convey
that the object behind it is "going to expire soon-ish..." and their resources may be reused. When
you construct an `rvalue` reference (specifically an `xvalue`) with `std::move`, you may pass this
reference around to, for instance, a a move constructor which can use the fact that the provided
reference will expire soon, so instead of copying the underlying data, the constructor can
efficiently steal the underlying data.

Note that the C++ compiler will not prevent you from using the moved-from object again, and it
**will** run the moved-from object's destructor. This means that moved-from objects must remain in
a valid state at all times.

One consequence of this design is that C++'s move semantics are significantly broader than Rust's
since C++ enables library authors to run custom logic when moving a value in memory.

Furthermore, C++ allows you to `= delete` a move constructor which effectively prohibits a value
from moving in memory once constructed

### Rust's Move Semantics

Meanwhile, Rust's move semantics are very simple. When you move a value, the Rust compiler will
effectively copy the struct's bits and invalidate the moved from object. The Rust compiler will not
run the `Drop` implementation on the moved-from value.

### Trivial Relocatability

Rust's move semantics can be summarized as "all objects in Rust are trivially relocatable", and for
the purpose of `zngur` interop, a C++ object is considered trivially relocatable if performing a
trivial relocatable move on the object has the same observable effect as move constructing the C++
object in the new memory location and running the moved-from objects destructor.

A lot of C++ objects likely follow this guarantee. A few examples likely include:

- Plain old data types
- any `T` with a `T(T&&) = default;` implementation where all members are trivially relocatable.
- `std::vector<T>` regardless of `T`'s trivial relocatability.
  - Most other collections
- `std::unique_ptr` and `std::shared_ptr`
- `std::future`

A few **negative** examples are:

- Self referential types
- `libstdc++`'s `std::string` since small-string optimization makes it self-referential
  - NOTE: `stdc++`'s implementation of `std::string` does not do small-string optimization. That is
    to say, this is very tricky stuff...

### C++ Trivially Relocatable in Rust

`zngur` allows you to own a C++ object in the Rust stack so long as the C++ object meets the
trivial relocatability guarantees. Let's go over an example (you can follow along the full example
in `examples/stack_owned/`, where `cpp_type.h` aliases `using MyCppWrapper = CppType;`):

```zng
// main.zng
#cpp_additional_includes "
#include <cpp_type.h>
"

type c++::MyCppWrapper {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
```

`#layout` gives the size and alignment of the C++ type, and `#trivially_relocatable` says that it
can be moved like a Rust value. With both, `zngur` stores it directly in the Rust stack.

Just like with C++ opaque objects, we can define functions on an `extern "C++"` block:

```zng
extern "C++" {
    safe fn create_cpp_type(i32, i32) -> c++::MyCppWrapper;
    safe fn print_cpp_type(&c++::MyCppWrapper);
}
```

The C++ code has access to the `.cpp()` method used to access the inner type.

```rust
// main.rs
mod generated;

fn main() {
    println!("Hello from Rust");
    let c = generated::create_cpp_type(10, 20);
    println!("Rust got CppType");
    generated::print_cpp_type(&c);
    println!("Rust dropping CppType");
}
```

The wrapper unconditionally implements the following traits defined in the `zngur-lib` crate.

- The marker `ZngCppObject`
- The marker `ZngCppStackObject`
- The `ZngCppDestruct` trait with an `unsafe fn destruct(&mut self)`.

In the future, we may add generic functionality around these traits such as safe
in-place construction.

### Type trait

`#trivially_relocatable` is checked in C++: the generated code `static_assert`s
`::rust::is_trivially_relocatable<T>`. That type trait is true for all `std::is_trivially_copyable`
types and for `std::unique_ptr` (with the default deleter). For other types, specialize it:

```c++
namespace rust {
template <> struct is_trivially_relocatable<CppType> : std::true_type {};
} // namespace rust
```

In a future where `std::is_trivially_relocatable` exists in the standard, we can specialize it for
all types.

## Heap-Allocated C++ Types

If a C++ type is not trivially relocatable, Rust can own a heap-allocated instance of it with
`#heap_allocated`:

```zng
type c++::cpp_inventory::Inventory {
    #heap_allocated;
}
```

In C++, `rust::cpp::cpp_inventory::Inventory::build(args...)` allocates a `::cpp_inventory::Inventory`
on the C++ heap via `rust::ZngurCppOpaqueOwnedObject::build<...>(args...)`, and `.cpp()` accesses the
underlying C++ reference. See
[`examples/tutorial_cpp`](https://github.com/HKalbasi/zngur/blob/main/examples/tutorial_cpp) for a
complete working example.

> **NOTE**: `#cpp_stack_owned`, `#cpp_ref`, `#cpp_heap_allocated` and `#cpp_value` were removed.
> zngur explains how to replace them when it finds them.
