#!/usr/bin/env bash
# Run Inspect and promptfoo on tiny evaluations with models that call no AI (Inspect's
# mockllm, promptfoo's echo provider) and keep their outputs as the adapter fixtures in
# tests/fixtures/harness/. Downloads inspect-ai from PyPI (uvx) and promptfoo from npm
# (npx); works in a temporary directory and turns promptfoo's telemetry and update
# checks off. Needs uv, npm and jq.
#
# Usage: scripts/gen_harness_fixtures.sh [inspect|promptfoo]...   (default: both)
set -euo pipefail

# The versions the committed fixtures come from (see tests/fixtures/harness/README.md).
INSPECT_VERSION=0.3.286
PROMPTFOO_VERSION=0.124.1

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/tests/fixtures/harness"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

inspect() {
    local d="$work/inspect"
    mkdir -p "$d" "$out/inspect"
    cp "$out/inspect/probe_task.py" "$d/"
    (cd "$d" && uvx --from "inspect-ai==${INSPECT_VERSION}" inspect eval probe_task.py --model mockllm/model \
        --log-dir logs >/dev/null 2>&1)
    (cd "$d" && uvx --from "inspect-ai==${INSPECT_VERSION}" inspect eval probe_task.py --model mockllm/model \
        --log-dir logs-json --log-format json >/dev/null 2>&1)
    cp "$d"/logs/*.eval "$out/inspect/probe.eval"
    cp "$d"/logs-json/*.json "$out/inspect/probe.json"
    echo "inspect-ai $(cd "$d" && uvx --from "inspect-ai==${INSPECT_VERSION}" python -c 'import inspect_ai; print(inspect_ai.__version__)')"
}

promptfoo() {
    local d="$work/promptfoo"
    mkdir -p "$d" "$out/promptfoo"
    cp "$out/promptfoo/promptfooconfig.yaml" "$d/"
    export PROMPTFOO_DISABLE_TELEMETRY=1 PROMPTFOO_DISABLE_UPDATE=1 PROMPTFOO_CONFIG_DIR="$d/.pf"
    # One test fails on purpose, so the exit code is not 0.
    (cd "$d" && npx -y "promptfoo@${PROMPTFOO_VERSION}" eval -c promptfooconfig.yaml -o "$d/out.json" \
        --repeat 2 --no-cache --no-progress-bar >/dev/null 2>&1) || true
    # The output records the working directory and the machine; keep neither.
    jq '.config.basePath = "." | .config.outputPath = ["out.json"]
        | .metadata.nodeVersion = "redacted" | .metadata.platform = "redacted"
        | .metadata.arch = "redacted"' "$d/out.json" >"$out/promptfoo/out.json"
    echo "promptfoo $(cd "$d" && npx -y "promptfoo@${PROMPTFOO_VERSION}" --version 2>/dev/null | tail -n 1)"
}

for h in "${@:-inspect promptfoo}"; do
    for one in $h; do
        "$one"
    done
done
