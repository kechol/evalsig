#!/bin/sh
# Cross-language check: build stores with the Rust binary over the examples, verify
# them with the independent Python verifier, and confirm the verifier rejects a
# rewritten ledger line and a rewritten report value.
# Usage: scripts/cross_verify.sh [path/to/evalsig]   (needs uv and jq)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-"$root/target/debug/evalsig"}
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
export EVALSIG_ACTOR=ci:cross-verify

verify() {
    uv run --quiet --locked --project "$root/scripts" "$root/scripts/verify_store.py" "$@"
}

"$bin" --project "$dir/synthetic" init >/dev/null
cp "$root/examples/synthetic/config.toml" "$dir/synthetic/.evalsig/config.toml"
"$bin" --project "$dir/synthetic" import "$root/examples/synthetic/records.jsonl" >/dev/null
"$bin" --project "$dir/synthetic" report triage-v2 >/dev/null
"$bin" --project "$dir/synthetic" compare triage-v1 triage-v2 >/dev/null

"$bin" --project "$dir/examples" init >/dev/null
printf '[project]\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n' \
    >"$dir/examples/.evalsig/config.toml"
"$bin" --project "$dir/examples" import "$root/examples/records.jsonl" >/dev/null
"$bin" --project "$dir/examples" report r-base >/dev/null
"$bin" --project "$dir/examples" report r-judge-score >/dev/null

for report in "$dir"/synthetic/.evalsig/reports/*.json; do
    "$bin" --project "$dir/synthetic" verify "$report" >/dev/null
done

verify "$dir/synthetic/.evalsig" "$dir/examples/.evalsig" \
    "$root/examples/ledger.jsonl" "$root/examples/ledger-agent.jsonl"

cp -R "$dir/synthetic/.evalsig" "$dir/tampered"
ledger="$dir/tampered/ledger.jsonl"
jq -c 'if .kind == "compare" then .summary.verdict = "worsened" else . end' "$ledger" >"$dir/ledger.tmp"
mv "$dir/ledger.tmp" "$ledger"
if verify "$dir/tampered" >/dev/null; then
    echo "cross-verify: a rewritten ledger line was not detected" >&2
    exit 1
fi

cp -R "$dir/synthetic/.evalsig" "$dir/tampered-report"
report=$(find "$dir/tampered-report/reports" -name '*.json' | head -n 1)
jq '.results.metrics[0].value = 0.5' "$report" >"$dir/report.tmp"
mv "$dir/report.tmp" "$report"
if verify "$dir/tampered-report" >/dev/null; then
    echo "cross-verify: a rewritten report value was not detected" >&2
    exit 1
fi

echo "cross-verify passed: Rust and Python agree on every hash; tampering is detected"
