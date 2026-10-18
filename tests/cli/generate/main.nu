#@ run-status: 0

use ./snapshot-utils.nu *

let zngur = $env.ZNGUR_BIN
let expected = $env.ZNGUR_TEST_SOURCE_DIR

# Default output paths are generated.{h,cpp} next to the input and
# src/generated.rs (which is why `src` must already exist).
mkdir src
let gen = (do { ^$zngur generate input.zng } | complete)
if $gen.exit_code != 0 {
    print $"FAIL: generate exited with ($gen.exit_code): ($gen.stderr)"
    exit 1
}
for file in ["generated.h" "generated.cpp" "src/generated.rs"] {
    if not ($file | path exists) {
        print $"FAIL: generate did not create ($file)"
        exit 1
    }
}
# `--depfile` lists the inputs that produced the outputs.
let dep = (do { ^$zngur generate input.zng --depfile deps.d } | complete)
if $dep.exit_code != 0 {
    print $"FAIL: generate --depfile exited with ($dep.exit_code): ($dep.stderr)"
    exit 1
}
if not ("deps.d" | path exists) {
    print "FAIL: generate --depfile did not create deps.d"
    exit 1
}
assert-snapshot "depfile" (open deps.d) ($expected | path join "deps.d.snapshot")
