# Autozng

Autozng generates the Zngur [`.zng` interface file](../tutorial.md) for a Rust crate automatically,
instead of writing it by hand.
It reads the crate's public interface and writes the declarations Zngur needs:
each type with its [layout](../call_rust_from_cpp/layout_policy.md)
and [well-known traits](../call_rust_from_cpp/wellknown_traits.md),
its fields, variants and methods, and the crate's free functions.

Writing the IDL by hand means copying every signature and looking up every size and alignment.
Autozng takes both from the compiler, so they keep matching the crate as it changes.

## The two halves

Autozng is split into two programs:

- **`autozng-driver`** is a compiler driver.
  It runs where `rustc` runs, and writes the `.zng` file for the crate it is compiling.
- **`cargo-autozng`** is the command line frontend.
  It runs `cargo check` with the driver installed as the compiler wrapper,
  so every crate in the workspace is inspected and gets its own `.zng` file.

In normal use you call the frontend, and it runs the driver for you.

## What it is useful for

- **Layout information without the guesswork.**
  Zngur needs the size and alignment of every type it stores by value.
  Autozng writes the real values it reads from the compiler,
  instead of the numbers you would otherwise copy from your editor and keep up to date.
  See [Layout policy](../call_rust_from_cpp/layout_policy.md) for when these values are safe to rely on.
- **Well-known trait detection.**
  Autozng notices the well-known traits a type implements
  (such as `Debug` and `Copy`, and `?Sized` for unsized types)
  and adds them to the generated `wellknown_traits(...)`.
  See [Well-known traits](../call_rust_from_cpp/wellknown_traits.md).
- **A complete public interface.**
  Types, fields, methods and free functions are exported according to their visibility,
  so the generated file matches what the crate actually offers.
  See [Default visibility](./visibility.md).
- **Interface types included automatically.**
  A type that an exported field or signature mentions is declared too,
  so the generated file is self-contained.

## In this section

- [Installation](./installation.md)
- [Usage](./usage.md)
- [Default visibility](./visibility.md)
- [Attributes](./attributes.md)
