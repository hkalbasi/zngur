use crate::{format_book, test};
use anyhow::{Context, Result};
use xshell::{Shell, cmd};

fn check_crate(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo check").run()?;
    cmd!(sh, "cargo fmt --check")
        .run()
        .with_context(|| "Crate is not formatted. Run `cargo fmt`")?;
    Ok(())
}

fn check_book_formatting() -> Result<()> {
    format_book::main(false /* don't fix */)
        .with_context(|| "Book markdown files are not formatted. Run `cargo xtask format-book`")
}

pub fn main(fix: bool) -> Result<()> {
    let sh = &Shell::new()?;
    println!("Cargo version = {}", cmd!(sh, "cargo --version").read()?);

    #[cfg(not(target_os = "windows"))]
    {
        let cxx = sh.var("CXX")?;
        println!("CXX version = {}", cmd!(sh, "{cxx} --version").read()?);
    }
    #[cfg(target_os = "windows")]
    let msvc_ver: u32 = {
        let msvc = String::from_utf8(cmd!(sh, "cl.exe /help").output()?.stderr)?;
        let msvc = msvc
            .lines()
            .next()
            .and_then(|line| line.split("Version ").nth(1))
            .unwrap_or_default();

        let mut parts = msvc.split(".");
        let major = parts.next().unwrap_or("0");
        let minor = parts.next().unwrap_or("0");
        let msvc_ver: u32 = format!("{major}{minor}").parse()?;

        println!("MSVC version = {msvc} ({msvc_ver})",);
        msvc_ver
    };

    #[cfg(not(target_os = "windows"))]
    sh.set_var("RUSTFLAGS", "-D warnings");

    #[cfg(target_os = "windows")]
    {
        // Prevent link.exe error:1318
        let link_debug = if msvc_ver > 1944 {
            // sorry, msvc 2026 removed FASTLINK support so it can't prevent pdb
            // corruption. best we can do is disable debug symbols
            "/DEBUG:NONE"
        } else {
            "/DEBUG:FASTLINK"
        };
        sh.set_var(
            "RUSTFLAGS",
            format!("-D warnings -C link-arg={link_debug} -C link-arg=/INCREMENTAL:NO"),
        );
    }

    if fix {
        cmd!(sh, "cargo fmt --all").run()?;
        if let Err(e) = format_book::main(true /* fix */) {
            eprintln!("Warning: Failed to format book: {}", e);
        }
    }

    #[cfg(not(target_os = "windows"))]
    cmd!(sh, "cspell .")
        .run()
        .with_context(|| "Failed to check word spellings")?;
    #[cfg(target_os = "windows")]
    cmd!(sh, "cspell.cmd .")
        .run()
        .with_context(|| "Failed to check word spellings")?;

    check_book_formatting().with_context(|| "Book formatting check failed")?;

    for dir in sh.read_dir(".")? {
        if sh.path_exists(dir.join("Cargo.toml")) {
            let _crate_dir = sh.push_dir(dir.file_name().unwrap_or_default());
            check_crate(sh).with_context(|| format!("Checking crate {dir:?} failed"))?;
        }
    }

    test::main(fix).with_context(|| "End-to-end tests and examples failed")?;

    // it's nigh impossible to get this to run under MSVC
    #[cfg(not(target_os = "windows"))]
    cmd!(sh, "cargo test --all-features").run()?;

    Ok(())
}
