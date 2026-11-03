#!/bin/sh
# End-to-end smoke test over the examples: init, import, report and compare in a
# temporary project, checking the verdict of the synthetic comparison.
# Usage: scripts/smoke_test.sh [path/to/evalsig]
set -eu
bin=${1:-target/debug/evalsig}
root=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
"$bin" --project "$dir" init >/dev/null
cp "$root/examples/synthetic/config.toml" "$dir/.evalsig/config.toml"
"$bin" --project "$dir" import "$root/examples/synthetic/records.jsonl" >/dev/null
"$bin" --project "$dir" --format json report triage-v2 >"$dir/report.json"
"$bin" --project "$dir" --format json compare triage-v1 triage-v2 >"$dir/compare.json"
verdict=$(sed -n 's/^    "code": "\([a-z_]*\)",$/\1/p' "$dir/compare.json" | head -n 1)
if [ "$verdict" != "improved" ]; then
    echo "smoke test: expected verdict improved, got '$verdict'" >&2
    exit 1
fi
echo "smoke test passed: compare verdict is $verdict"
