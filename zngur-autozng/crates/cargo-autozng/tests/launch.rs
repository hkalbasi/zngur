//! The pure parts of launching: the loader path, the driver search order, and the toolchain
//! derivation. None of these need a toolchain installed, only paths.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use cargo_autozng::launch::{driver_path, dylib};
use cargo_autozng::toolchain::{self, Toolchain, ToolchainSource};

#[test]
fn the_loader_directory_is_bin_on_windows_and_lib_elsewhere() {
    let sysroot = Path::new("/toolchain");
    if cfg!(windows) {
        assert_eq!(toolchain::dylib_dir(sysroot), sysroot.join("bin"));
    } else {
        assert_eq!(toolchain::dylib_dir(sysroot), sysroot.join("lib"));
    }
}

#[test]
fn prepending_a_directory_keeps_the_existing_entries_after_it() {
    let dir = Path::new("/toolchain/lib");
    let expected = dir.as_os_str().to_os_string();

    assert_eq!(dylib::prepend(dir, None), expected);
    assert_eq!(dylib::prepend(dir, Some(OsStr::new(""))), expected);

    let existing = OsStr::new("/first");
    let joined = dylib::prepend(dir, Some(existing));
    let entries: Vec<PathBuf> = std::env::split_paths(&joined).collect();
    assert_eq!(entries, [PathBuf::from(dir), PathBuf::from(existing)]);
}

#[test]
fn the_driver_search_order_prefers_explicit_then_sibling_then_workspace_then_path() {
    let name = driver_path::driver_file_name();
    let order = driver_path::search_order(
        Some(PathBuf::from("/explicit/driver")),
        Some(PathBuf::from("/app/bin/cargo-autozng")),
        Some(PathBuf::from("/workspace")),
    );

    assert_eq!(order[0], PathBuf::from("/explicit/driver"));
    assert_eq!(order[1], Path::new("/app/bin").join(&name));
    assert_eq!(order[2], Path::new("/workspace/target/release").join(&name));
    assert_eq!(order[3], Path::new("/workspace/target/debug").join(&name));
    assert_eq!(order[4], PathBuf::from(&name));
}

#[test]
fn the_search_order_degrades_to_the_bare_name() {
    let name = driver_path::driver_file_name();
    let order = driver_path::search_order(None, None, None);
    assert_eq!(order, [PathBuf::from(&name)]);
}

#[test]
fn a_managed_toolchain_forces_the_channel_and_an_ambient_one_does_not() {
    let managed = Toolchain::derive(Some("nightly-2026-08-29"), PathBuf::from("/tc"), "v".into());
    assert_eq!(
        managed.source,
        ToolchainSource::Managed {
            channel: "nightly-2026-08-29".to_string()
        }
    );
    assert_eq!(managed.sysroot, PathBuf::from("/tc"));
    assert_eq!(managed.dylib_dir, toolchain::dylib_dir(Path::new("/tc")));
    assert_eq!(
        managed.launch_env(),
        [("RUSTUP_TOOLCHAIN", OsString::from("nightly-2026-08-29"))]
    );

    let ambient = Toolchain::derive(None, PathBuf::from("/tc"), "v".into());
    assert_eq!(ambient.source, ToolchainSource::Ambient);
    assert!(ambient.launch_env().is_empty());
}
