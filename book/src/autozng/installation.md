# Installation

Autozng is not published on crates.io yet, so install it from the repository.

## Install the frontend

Clone the repository and install the frontend from its directory in the checkout:

```console
git clone https://github.com/hkalbasi/zngur
cargo install --path zngur/zngur-autozng/crates/cargo-autozng
```

This installs the `cargo-autozng` binary,
which also gives you the `cargo autozng` subcommand.

## Build the driver

The driver runs inside the compiler, so it needs a nightly toolchain with the
`rustc-dev` and `llvm-tools` components.
The checkout pins a matching nightly in `zngur-autozng/rust-toolchain.toml`,
so `rustup` selects it for you.
Build the driver from the `zngur-autozng` directory:

```console
cd zngur/zngur-autozng
cargo build -p autozng-driver
```

The binary is written to `target/debug/autozng-driver`
(use `cargo build --release -p autozng-driver` for `target/release/autozng-driver`).

## Point the frontend at the driver

The frontend does not build the driver itself, so tell it where the binary is.
Use the `--driver` option:

```console
cargo autozng --driver /path/to/zngur/zngur-autozng/target/debug/autozng-driver
```

or set `AUTOZNG_DRIVER` once:

```console
export AUTOZNG_DRIVER=/path/to/zngur/zngur-autozng/target/debug/autozng-driver
```

The frontend also looks for the driver next to itself, in the checkout's
`target/release` and `target/debug`, and on `PATH`,
so `--driver` is only needed when the two live apart.

The frontend reads the compiler version the driver was built for
and runs your crate with that toolchain,
so the driver's pinned nightly is used for the analysis even if your project uses another one.
