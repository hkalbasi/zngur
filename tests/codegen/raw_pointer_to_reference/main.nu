#@ run-status: 0

# Generate a header whose IDL names `*const &T` / `*mut &mut T`, then compile a C++
# translation unit that uses those methods. The build treats warnings as errors
# (`-Werror` on GCC/Clang, `/WX` on MSVC), so a missing `__zngur_internal`
# specialization for `Raw<Ref<T>>` / `RawMut<RefMut<T>>` fails here instead of only
# warning in downstream users.

let zngur = $env.ZNGUR_BIN

mkdir src
let generate = (do {
    ^$zngur generate raw_pointer_to_reference.zng -i --crate-name crate --rs-file src/generated.rs --h-file generated.h --cpp-file generated.cpp
} | complete)
if $generate.exit_code != 0 {
    print $"FAIL: zngur generate exited with ($generate.exit_code)"
    print $generate.stderr
    exit 1
}

let os = $nu.os-info.name
if $os == "windows" {
    let cxx = (do {
        run-external "cl.exe" "/nologo" "/W4" "/WX" "/EHsc" "/std:c++20" "/Zc:__cplusplus" "/I." "/c" "use.cpp" "/Fo:use.obj"
    } | complete)
    if $cxx.exit_code != 0 {
        print $"FAIL: cl.exe exited with ($cxx.exit_code)"
        print $cxx.stderr
        exit 1
    }
} else {
    let cxx = (do {
        run-external "c++" "-std=c++20" "-Werror" "-I." "-c" "use.cpp" "-o" "use.o"
    } | complete)
    if $cxx.exit_code != 0 {
        print $"FAIL: c++ exited with ($cxx.exit_code)"
        print $cxx.stderr
        exit 1
    }
}
