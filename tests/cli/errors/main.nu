#@ run-status: 0

use ./snapshot-utils.nu *

let zngur = $env.ZNGUR_BIN
let expected = $env.ZNGUR_TEST_SOURCE_DIR

# An invalid `.zng` is rejected with a diagnostic pointing at the source span.
# Strip the ANSI styling and the trailing panic report (whose location and
# thread id can change) so the snapshot covers just the diagnostic.
let bad = (do { ^$zngur generate bad.zng } | complete)
if $bad.exit_code == 0 {
    print "FAIL: generate accepted an invalid file"
    exit 1
}
if ($bad.stdout | str trim) != "" {
    print $"FAIL: unexpected stdout: ($bad.stdout)"
    exit 1
}
let cleaned = ($bad.stderr
    | ansi strip
    | str replace --regex '(?s)thread .*' ""
    | str trim)
assert-snapshot "invalid directive" $cleaned ($expected | path join "bad.stderr.snapshot")
