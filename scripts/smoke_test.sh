#!/bin/sh
# End-to-end smoke test over the examples: init, import, report and compare in a
# temporary project, checking the report's metrics and the comparison's verdict.
# Usage: scripts/smoke_test.sh [path/to/evalsig]   (needs jq)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-"$root/target/debug/evalsig"}
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
"$bin" --project "$dir" init >/dev/null
cp "$root/examples/synthetic/config.toml" "$dir/.evalsig/config.toml"
"$bin" --project "$dir" import "$root/examples/synthetic/records.jsonl" >/dev/null
"$bin" --project "$dir" --format json report triage-v2 >"$dir/report.json"
"$bin" --project "$dir" --format json compare triage-v1 triage-v2 >"$dir/compare.json"
metrics=$(jq -r '[.results.metrics[].metric] | join(",")' "$dir/report.json")
if [ "$metrics" != "accuracy,hit_rate,false_alarm_rate,precision" ]; then
    echo "smoke test: unexpected report metrics '$metrics'" >&2
    exit 1
fi
verdict=$(jq -r '.verdict.code' "$dir/compare.json")
if [ "$verdict" != "improved" ]; then
    echo "smoke test: expected verdict improved, got '$verdict'" >&2
    exit 1
fi
echo "smoke test passed: report has $metrics; compare verdict is $verdict"
