#@ run-status: 0

use ./snapshot-utils.nu *

# Build the png-reexport example as a staticlib with its generated bridge, then compile the C++
# test against generated.h and link the staticlib. The C++ test decodes quadrants.png through
# the generated interface and prints the pixel corners where the colors change.

let root = $env.ZNGUR_ROOT
let source = $env.ZNGUR_TEST_SOURCE_DIR
let example = ($root | path join "zngur-autozng" "examples" "png-reexport")
let crate = ($env.ZNGUR_TEST_DIR | path join "crate")
let zng_dir = ($env.ZNGUR_TEST_DIR | path join "zng")
let target = ($crate | path join "target" "debug")

mkdir $zng_dir ($crate | path join "src")

# 1. Generate the .zng from the example with the autozng frontend.
let frontend_bin = $env.AUTOZNG_FRONTEND
let driver_bin = $env.AUTOZNG_DRIVER
let frontend = (do {
    ^$frontend_bin --driver $driver_bin --output-dir $zng_dir --target-dir ($env.ZNGUR_TEST_DIR | path join "target" "zng") --manifest-path ($example | path join "Cargo.toml")
} | complete)
if $frontend.exit_code != 0 {
    print $"FAIL: cargo-autozng exited with ($frontend.exit_code)"
    print $frontend.stderr
    exit 1
}

let zng = ($zng_dir | path join "png_reexport.zng")
if not ($zng | path exists) {
    print $"FAIL: cargo-autozng did not write ($zng)"
    exit 1
}

# Every type gets exactly one block: records are keyed by the region-erased type, so a lifetime
# parameter must not duplicate `Info<'a>` / `Info<'_>`.
let type_headers = (open --raw $zng | lines | where { |line| $line | str starts-with "type " })
let duplicates = ($type_headers | group-by | transpose name blocks | where { |row| ($row.blocks | length) > 1 })
if ($duplicates | is-not-empty) {
    print "FAIL: duplicate type blocks in the generated interface:"
    print ($duplicates | select name | to text)
    exit 1
}

# 2. Generate the bridge sources.
let zngur = $env.ZNGUR_BIN
let bridge = (do {
    ^$zngur generate $zng -i --crate-name crate --rs-file ($crate | path join "src" "generated.rs") --h-file ($crate | path join "generated.h") --cpp-file ($crate | path join "generated.cpp")
} | complete)
if $bridge.exit_code != 0 {
    print $"FAIL: zngur generate exited with ($bridge.exit_code)"
    print $bridge.stderr
    exit 1
}

# 3. Guard the walk budgets. The unmodified defaults once produced a 154 MB header for this
#    interface; the current (6,1,1) budgets plus the free std hop keep it around 12 MB. A
#    regression here must fail the test rather than merely slow the build down.
let header = ($crate | path join "generated.h")
let header_size = (ls $header | get 0.size)
if $header_size > 20MB {
    print $"FAIL: generated.h is ($header_size), over the 20 MB limit"
    exit 1
}

# 4. Assemble the scratch crate. It sits outside the zngur-autozng workspace, so the copied
#    manifest gains an empty [workspace] table.
cp ($example | path join "Cargo.toml") ($crate | path join "Cargo.toml")
cp ($example | path join "src" "lib.rs") ($crate | path join "src" "lib.rs")
cp ($example | path join "test" "vertices.cpp") "vertices.cpp"
cp ($example | path join "testdata" "quadrants.png") "quadrants.png"

let manifest = "[workspace]\n\n" + (open --raw ($crate | path join "Cargo.toml"))
$manifest | save -f ($crate | path join "Cargo.toml")

# 5. Build the staticlib.
let build = (do {
    ^cargo build --manifest-path ($crate | path join "Cargo.toml")
} | complete)
if $build.exit_code != 0 {
    print $"FAIL: cargo build exited with ($build.exit_code)"
    print $build.stderr
    exit 1
}

# 6. Compile and link the C++ test.
let generated_cpp = ($crate | path join "generated.cpp")
let os = $nu.os-info.name
if $os == "windows" {
    let args = [
        "/nologo" "/W4" "/EHsc" "/std:c++20" "/Zc:__cplusplus"
        $"/I($crate)" "vertices.cpp"
    ]
    let args = if ($generated_cpp | path exists) { $args | append $generated_cpp } else { $args }
    let args = $args | append [
        "/Fe:vertices.exe" "/link" "ntdll.lib" "Ws2_32.lib" "Userenv.lib"
        ($target | path join "png_reexport.lib") $"/LIBPATH:($target)"
    ]
    let cxx = (do { run-external "cl.exe" ...$args } | complete)
    if $cxx.exit_code != 0 {
        print $"FAIL: cl.exe exited with ($cxx.exit_code)"
        print $cxx.stderr
        exit 1
    }
} else {
    let args = if ($generated_cpp | path exists) {
        ["-std=c++20" "-Werror" $"-I($crate)" "vertices.cpp" $generated_cpp]
    } else {
        ["-std=c++20" "-Werror" $"-I($crate)" "vertices.cpp"]
    }
    let args = $args | append ["-L" $target "-l" "png_reexport" "-o" "vertices"]
    let cxx = (do { run-external "c++" ...$args } | complete)
    if $cxx.exit_code != 0 {
        print $"FAIL: c++ exited with ($cxx.exit_code)"
        print $cxx.stderr
        exit 1
    }
}

# 7. Run it and compare the vertex list with the fixture.
let exe = if $os == "windows" { "vertices.exe" } else { "vertices" }
let run = (do { ^($env.ZNGUR_TEST_DIR | path join $exe) } | complete)
if $run.exit_code != 0 {
    print $"FAIL: ($exe) exited with ($run.exit_code)"
    print $run.stdout
    print $run.stderr
    exit 1
}

let output = $run.stdout | str replace --all "\r\n" "\n"
assert-snapshot "png-reexport vertices" $output ($source | path join "vertices.output")
