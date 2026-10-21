# Usage

## Generating the interface

Run the frontend from the crate or workspace you want to inspect:

```console
cargo autozng
```

It runs `cargo check` with the driver installed as the compiler wrapper,
so each crate that is built gets its own `.zng` file.

Everything the frontend does not recognise is passed to `cargo check` unchanged,
so the usual cargo flags work:

```console
cargo autozng --workspace --features extra -p my_crate
```

## Where the files are written

By default the `.zng` files go to `<target>/autozng`,
where `<target>` is your workspace's target directory (`target` unless you configured otherwise).
Each crate gets a file named after it, so the crate `my_crate` produces `target/autozng/my_crate.zng`.

Two options change the destination:

- `-o <DIR>` / `--output-dir <DIR>` writes the files to `DIR` instead.
- `--output-zng <FILE>` writes the interface to a single file.
  It is meant for a build that produces one crate.

The wrapped build gets its own target directory (`<target>/autozng`),
so running Autozng does not interfere with your normal builds.
`--target-dir <DIR>` overrides it.

## Useful options

| Option            | Effect                                                  |
| ----------------- | ------------------------------------------------------- |
| `--driver <PATH>` | Use the driver at `PATH`.                               |
| `--release`       | Build with the `release` profile.                       |
| `-v`, `--verbose` | Print the driver discovery steps and the cargo command. |
| `--dry-run`       | Print what would run, without running it.               |
| `-h`, `--help`    | Show the frontend's help.                               |

## Running the driver directly

The frontend is a convenience wrapper; the driver is the program that does the work.
It behaves like `rustc`, so it takes the arguments you would give the compiler,
plus `--output-zng <FILE>` to choose where the interface is written:

```console
autozng-driver --edition=2024 --crate-type=lib --crate-name=my_crate \
    src/lib.rs --output-zng my_crate.zng
```

The driver turns on its own attributes for the compilation,
analyses the crate, and writes the interface for the crate it just saw.
Run this way it must also be able to find its compiler's libraries;
the frontend sets that up for you, which is why `cargo autozng` is the recommended way to use Autozng.
