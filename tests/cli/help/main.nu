#@ run-status: 0

use ./snapshot-utils.nu *

let zngur = $env.ZNGUR_BIN
let expected = $env.ZNGUR_TEST_SOURCE_DIR

# `--version` prints the crate version.
let version = (do { ^$zngur --version } | complete)
if $version.exit_code != 0 {
    print $"FAIL: `--version` exited with ($version.exit_code): ($version.stderr)"
    exit 1
}
if ($version.stderr | str trim) != "" {
    print $"FAIL: `--version` wrote to stderr: ($version.stderr)"
    exit 1
}
let version = ($version.stdout | str replace --regex r#'\d+\.\d+\.\d+.*'# "VERSION")
assert-snapshot "version" $version ($expected | path join "version.stdout.snapshot")

# `--help` for the top-level command and each subcommand.
let helps = [
    { name: "help", args: ["--help"], file: "help.stdout.snapshot" }
    { name: "generate --help", args: ["generate", "--help"], file: "generate.help.stdout.snapshot" }
    { name: "make-zng-header --help", args: ["make-zng-header", "--help"], file: "make-zng-header.help.stdout.snapshot" }
]
for help in $helps {
    let out = (do { ^$zngur ...$help.args } | complete)
    if $out.exit_code != 0 {
        print $"FAIL: `($help.name)` exited with ($out.exit_code): ($out.stderr)"
        exit 1
    }
    if ($out.stderr | str trim) != "" {
        print $"FAIL: `($help.name)` wrote to stderr: ($out.stderr)"
        exit 1
    }
    let normalized = ($out.stdout | str replace --all "zngur.exe" "zngur")
    assert-snapshot $help.name $normalized ($expected | path join $help.file)
}
