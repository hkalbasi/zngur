# End-to-end tests and examples

Beside the examples (which must compile and stay advertised), these tests cover
cases that should be rejected and patterns we don't want in the examples.

Run them with:

```sh
cargo xtask test                               # check
cargo xtask test --fix                         # create or update the snapshots
cargo xtask test 'tests/parser/*'              # run tests matching a glob
cargo xtask test 'examples/simple/*'           # run a matching example
cargo xtask test '**/*safe*' -v                # print each passing test
cargo xtask test --filter-file fail_list.txt   # run previously failed tests
cargo xtask test --only-autozng                # only the autozng cases and suite
cargo xtask test --require-autozng             # fail if the autozng toolchain is missing
```

The glob is matched against paths relative to the repository root. A run that
fails prints the names again at the end and writes them to `fail_list.txt`, so
`cargo xtask test --filter-file fail_list.txt` reruns just those tests.

Each test is a `*.zng`, `*.rs`, or `main.nu` file whose first line is a `//@`
directive that selects the mode.

| Mode                | Meaning                                                         |
| ------------------- | --------------------------------------------------------------- |
| `//@ fail`          | `zngur generate` must reject it. The diagnostics, including     |
|                     | the source spans, are compared with the sibling `*.stderr`      |
|                     | snapshot.                                                       |
| `//@ compile_fail`  | Bridge generation must succeed, but compiling the sibling       |
|                     | `*.rs` and `*.cpp` must fail. Add `//@ error_contains: MESSAGE` |
|                     | (and optionally `//@ error_code: CODE`).                        |
| `//@ pass`          | The sibling `*.cpp` is linked into a binary built from the      |
|                     | sibling `*.rs`, and the binary must run successfully.           |
|                     | Its combined output is compared with `*.output`; generation     |
|                     | warnings use `*.stderr`.                                        |
| `//@ autozng`       | The `.rs` is a normal Rust `lib.rs`-style main module file.     |
|                     | The runner snapshots the generated `.zng` and `autozng:`        |
|                     | diagnostics, then generates and compiles the bridge with the    |
|                     | stable toolchain.                                               |
| `//@ autozng_fail`  | The driver must fail. Requires `//@ error_contains: MESSAGE`    |
|                     | and optionally `//@ error_code: CODE`; no snapshots.            |
| `//@ cargo_autozng` | Marks an example crate. Uses the frontend to generate the       |
|                     | `.zng`, every `//@ contains: TEXT` marker must be present,      |
|                     | the bridge is then compiled.                                    |

## autozng cases

A `//@ autozng` fixture is a normal Rust `lib.rs`-style main module file. The
runner writes it to `src/lib.rs` with the tool-registration prelude and runs the
driver.

`<case>.zng` is required; `<case>.stderr` holds the filtered `autozng:`
diagnostics and is written only when they are non-empty.

`//@ no_stderr` skips the diagnostics snapshot (temporary useful).

The autozng modes need the pinned nightly with `rustc-dev`:

```sh
rustup toolchain install nightly-2026-08-29 --profile minimal \
  --component rustc-dev --component llvm-tools --component rustfmt
```

Alternatively, point `AUTOZNG_TEST_SYSROOT` at an existing toolchain root
containing `bin/cargo`. Without either, autozng tests are skipped unless
`--require-autozng` is set.

## Free-form (Nushell) tests

Anything that doesn't fit the currently supported modes by the test runner is
written as a directory containing a `main.nu`. The harness copies the directory
into a scratch dir under `target/nu-tests/`, copies `tests/snapshot-utils.nu`
alongside it, and runs `nu main.nu` there, so the script may create files freely.
The script must exit `0`, or the status given by a `#@ run-status: <n>` directive
at the top of `main.nu`.

A Nushell test under `tests/autozng/` is an autozng test: it runs under
`cargo xtask test --only-autozng` and in addition to the usual variables gets
`AUTOZNG_DRIVER` and `AUTOZNG_FRONTEND` pointing at the built binaries. Use it
when a test needs to build a generated bridge itself (for example, linking it
into a C++ program).

If you find yourself adding nushell tests with similar runner script,
consider expanding the test runner and adding a new mode to it.
Nushell tests are for tests which doesn't follow a predefined form and are unique in their own way.
