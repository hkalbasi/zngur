/// Entry point for the `cargo autozng` / `cargo-autozng` command.
///
/// Planned flow (mirrors `cargo-clippy`):
/// 1. strip `env::args` so both `cargo autozng ...` and `cargo-autozng ...` normalize,
/// 2. locate/build the [`autozng-driver`](../autozng_driver/index.html) executable,
/// 3. re-invoke `cargo` with `RUSTC_WORKSPACE_WRAPPER=<autozng-driver>` so the driver runs
///    as the compiler wrapper for workspace crates.
///
/// TODO(cargo-autozng): forward the autozng output selection to the driver. The driver
/// already accepts `--output-zng <path>` and reads rustc's `--out-dir`, but a workspace
/// wrapper is invoked by cargo with only the rustc arguments, so those flags need an
/// out-of-band channel (e.g. an environment variable) to reach it.
pub fn run() -> Result<i32, String> {
    // strip env::args to normalize calling via `cargo-autozng` or `cargo autozng`
    // then dispatch then ensure driver is in place
    // then dispatch driver call
    todo!()
}
