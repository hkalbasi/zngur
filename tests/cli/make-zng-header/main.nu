#@ run-status: 0

let zngur = $env.ZNGUR_BIN

# The generated header guards its definitions and, by default, has no
# panic-to-exception machinery.
let plain = (do { ^$zngur make-zng-header zngur.h } | complete)
if $plain.exit_code != 0 {
    print $"FAIL: make-zng-header exited with ($plain.exit_code): ($plain.stderr)"
    exit 1
}
if not ("zngur.h" | path exists) {
    print "FAIL: make-zng-header did not create zngur.h"
    exit 1
}
let plain = (open zngur.h)
if not ($plain | str contains "#ifndef __ZNG_HEADER_INTERNAL_TYPES") {
    print "FAIL: header is missing its include guard"
    exit 1
}
if ($plain | str contains "__zngur_mark_panicked") {
    print "FAIL: header unexpectedly contains panic-to-exception machinery"
    exit 1
}

# `--panic-to-exception` adds the panic bookkeeping helpers.
let with_panic = (do { ^$zngur make-zng-header zngur_panic.h --panic-to-exception } | complete)
if $with_panic.exit_code != 0 {
    print $"FAIL: make-zng-header --panic-to-exception exited with ($with_panic.exit_code): ($with_panic.stderr)"
    exit 1
}
if not ((open zngur_panic.h) | str contains "__zngur_mark_panicked") {
    print "FAIL: --panic-to-exception did not add the panic helpers"
    exit 1
}

# `--cpp-namespace` wraps the generated types in the requested namespace.
let with_ns = (do { ^$zngur make-zng-header zngur_ns.h --cpp-namespace my_ns } | complete)
if $with_ns.exit_code != 0 {
    print $"FAIL: make-zng-header --cpp-namespace exited with ($with_ns.exit_code): ($with_ns.stderr)"
    exit 1
}
if not ((open zngur_ns.h) | str contains "namespace my_ns") {
    print "FAIL: --cpp-namespace did not apply the namespace"
    exit 1
}
