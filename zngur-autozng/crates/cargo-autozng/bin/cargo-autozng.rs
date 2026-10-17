//! Entrypoint for `cargo-autozng`.
//! Logic lives in the library at [`cargo_autozng::run::run`].

use std::process;

fn main() {
    match cargo_autozng::run::run() {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("cargo-autozng: {err}");
            process::exit(1);
        }
    }
}
