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

uv sync --quiet --locked --project "$root/scripts"
verify() {
    uv run --quiet --no-sync --project "$root/scripts" "$root/scripts/verify_store.py" "$@"
}

# Rewrite one file of a copy of the synthetic store with a jq filter and require the
# verifier to reject the copy with the expected problem.
expect_rejected() {
    what=$1 file=$2 filter=$3 flags=$4 expected=$5
    copy="$dir/tampered"
    rm -rf "$copy"
    cp -R "$dir/synthetic/.evalsig" "$copy"
    target="$copy/$file"
    # shellcheck disable=SC2086 # $flags is a list of jq options
    jq $flags "$filter" "$target" >"$dir/rewritten"
    mv "$dir/rewritten" "$target"
    if out=$(verify "$copy"); then
        echo "cross-verify: $what was not detected" >&2
        exit 1
    fi
    case $out in
    *"$expected"*) ;;
    *)
        printf 'cross-verify: %s was rejected for another reason:\n%s\n' "$what" "$out" >&2
        exit 1
        ;;
    esac
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

expect_rejected "a rewritten ledger line" ledger.jsonl \
    'if .kind == "compare" then .summary.verdict = "worsened" else . end' -c \
    "line 3: hash does not match the content"
report=$(cd "$dir/synthetic/.evalsig" &&
    for f in reports/*.json; do
        if [ "$(jq -r .schema "$f")" = evalsig.report/1 ]; then echo "$f"; fi
    done)
expect_rejected "a rewritten report value" "$report" '.results.metrics[0].value = 0.5' "" \
    "results hash to"

echo "cross-verify passed: Rust and Python agree on every hash; tampering is detected"
