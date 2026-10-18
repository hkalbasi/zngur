# Snapshot helpers shared by the free-form Nushell tests. The test harness
# copies this file into each test's scratch directory.

def diff [a: string, b: string] {
    let f1 = (mktemp)
    let f2 = (mktemp)
    $a | save -f $f1
    $b | save -f $f2
    ^diff $f1 $f2
}

# Compare `actual` with the snapshot at `snapshot_path`. When
# ZNGUR_UPDATE_SNAPSHOTS is set (the harness' `--fix` flag) the snapshot is
# rewritten instead of compared.
export def assert-snapshot [name: string, actual: string, snapshot_path: string] {
    if "ZNGUR_UPDATE_SNAPSHOTS" in $env {
        print $"updating snapshot: ($snapshot_path)"
        let content = if ($actual | str ends-with (char newline)) { $actual } else { $actual ++ (char newline) }
        let content = $content | str replace --all --regex "(?m)[ \t]+$" ""
        $content | save -f $snapshot_path
        return
    }
    let snapshot = (open $snapshot_path | str replace --all --regex "(?m)[ \t]+$" "" | str trim)
    if ($actual | str replace --all --regex "(?m)[ \t]+$" "" | str trim) != $snapshot {
        print $"FAIL: ($name) mismatch!"
        print "--- GOT ---"
        print $actual
        print "--- EXPECTED ---"
        print $snapshot
        print "--- Diff ---"
        diff $snapshot $actual
        exit 1
    }
}
