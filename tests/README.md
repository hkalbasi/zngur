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
```

The glob is matched against paths relative to the repository root. A run that
fails prints the names again at the end and writes them to `fail_list.txt`, so
`cargo xtask test --filter-file fail_list.txt` reruns just those tests.

Each test is a `*.zng`, `*.rs` or `main.nu` file whose first line is a `//@` directive which selects the mode.

- `//@ fail`: `zngur generate` must reject it. The diagnostics, including the
  source spans, are compared with the sibling `*.stderr` snapshot.
- `//@ compile_fail`: bridge generation must succeed, but compiling the sibling
  `*.rs` and `*.cpp` must fail. Add an `//@ error_contains: MESSAGE`
  annotation and optionally `//@ error_code: CODE` when the
  expected diagnostic has an error code.
- `//@ pass`: the sibling `*.cpp` is linked into a binary built from the sibling
  `*.rs`, and the binary must run successfully. Its combined stdout and
  stderr are compared with the sibling `*.output` snapshot. If generating the
  bridge emits warnings (e.g. from a deprecated item), they are compared with the
  sibling `*.stderr` snapshot. Both snapshots must exist whenever there is
  output.

## Free-form (Nushell) tests

Anything that doesn't fit the currently supported modes by the test runner is
written as a directory containing a `main.nu`. The harness copies the directory
into a scratch dir under `target/nu-tests/`, copies `tests/snapshot-utils.nu`
alongside it, and runs `nu main.nu` there, so the script may create files freely.
The script must exit `0`, or the status given by a `#@ run-status: <n>` directive
at the top of `main.nu`.

If you find yourself adding nushell tests with similar runner script,
consider expanding the test runner and adding a new mode to it.
Nushell tests are for tests which doesn't follow a predefined form and are unique in their own way.
