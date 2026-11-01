use zngur::Zngur;

fn main() {
    build::rerun_if_changed("main.zng");
    build::rerun_if_changed("impls.cpp");
    build::rerun_if_changed("src/");

    let crate_dir = build::cargo_manifest_dir();

    Zngur::from_zng_file(crate_dir.join("main.zng"))
        .with_cpp_file(crate_dir.join("generated.cpp"))
        .with_h_file(crate_dir.join("generated.h"))
        .with_rs_file(crate_dir.join("./src/generated.rs"))
        .with_crate_name("crate")
        // Exercises the destructor bridge of C++ types held by value with a mangling base.
        .with_mangling_base("stack_owned")
        .with_zng_header("zngur.h")
        .generate();

    let mut my_build = cc::Build::new();
    my_build.cpp(true).std("c++11").include(".");

    my_build.file("generated.cpp").compile("zngur_generated");
    my_build.file("impls.cpp").compile("impls_cpp");
}
