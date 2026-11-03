#!/bin/sh
# gate serve and gate submit across two OS users (SPEC §8.5, D-014). Run as root on
# a disposable Linux machine or container: it creates the users `referee` and
# `agent`, gives the referee the labels, the key and the ledger, gives the agent the
# remaining items only, and submits over the Unix socket and token HTTP.
# Usage: scripts/gate_two_users.sh path/to/evalsig   (scripts/gate_serve_docker.sh
# runs it in a container)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
[ "$(id -u)" = 0 ] || {
    echo "run as root" >&2
    exit 64
}

fail() {
    echo "gate two users: $*" >&2
    exit 1
}
# The value of a top-level-ish JSON string field in evalsig's output.
field() { sed -n "s/.*\"$1\": *\"\\([^\"]*\\)\".*/\\1/p" | head -n 1; }

groupadd -f gatesub
id referee >/dev/null 2>&1 || useradd -m -s /bin/sh referee
id agent >/dev/null 2>&1 || useradd -m -s /bin/sh -G gatesub agent
as() {
    u=$1
    shift
    su -s /bin/sh "$u" -c "EVALSIG_ACTOR=user:$u $*"
}

work=$(mktemp -d)
chmod 755 "$work"
# A copy both users can run, wherever the build lives.
bin="$work/evalsig"
cp "$1" "$bin"
chmod 755 "$bin"
ref="$work/referee"
home="$work/referee-home"
agent="$work/agent"
mkdir "$ref" "$home" "$agent"
cp "$root/examples/synthetic/records.jsonl" "$root/examples/synthetic/config.toml" "$work/"
chmod 644 "$work/records.jsonl" "$work/config.toml"
chown referee "$ref" "$home"
chown agent "$agent"

# The referee: all records, the identity, the holdout.
as referee "'$bin' --project '$ref' init >/dev/null && cp '$work/config.toml' '$ref/.evalsig/config.toml'"
as referee "'$bin' --project '$ref' import '$work/records.jsonl' >/dev/null"
as referee "'$bin' referee init '$home' >/dev/null"
holdout=$(as referee "'$bin' --project '$ref' --format json split --holdout --referee '$home' --out '$ref/remaining.json'" | field hash)
[ -n "$holdout" ] || fail "no holdout hash"
cp "$ref/remaining.json" "$work/remaining.json"
chmod 644 "$work/remaining.json"

# The agent: records without the holdout's labels.
tr ',' '\n' <"$work/remaining.json" | grep -o '"t-[0-9]*"' | tr -d '"' | sort >"$work/kept"
while IFS= read -r line; do
    case $line in
    *'"record":"label"'*)
        item=$(printf '%s' "$line" | sed -n 's/.*"item_id":"\([^"]*\)".*/\1/p')
        grep -qx "$item" "$work/kept" && printf '%s\n' "$line"
        ;;
    *) printf '%s\n' "$line" ;;
    esac
done <"$work/records.jsonl" >"$work/agent-records.jsonl"
chmod 644 "$work/agent-records.jsonl"
grep '"run_id":"triage-v2"' "$work/records.jsonl" >"$work/candidate.jsonl"
chmod 644 "$work/candidate.jsonl"
as agent "'$bin' --project '$agent' init >/dev/null && cp '$work/config.toml' '$agent/.evalsig/config.toml'"
as agent "'$bin' --project '$agent' import '$work/agent-records.jsonl' >/dev/null"
chmod -R a+rX "$agent"

cat >"$work/gate.toml" <<'TOML'
name = "triage-v2 is no worse"
hypothesis = "The new prompt keeps accuracy"
split = "holdout"
min_items = 10

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "not_worse_than"
value = -0.3
sided = "one"

[holdout]
budget = 3
TOML
chmod 644 "$work/gate.toml"

as referee "'$bin' --project '$ref' seal --holdout '$holdout' --referee '$home' --agent-project '$agent' >/dev/null"
draft=$(as agent "'$bin' --project '$agent' --format json preregister --draft '$work/gate.toml'" | field draft)
approved=$(as referee "'$bin' --project '$ref' --format json preregister approve '$draft' --holdout '$holdout' --from '$agent' --referee '$home'" | field approved)
[ -n "$approved" ] || fail "no approval"

# The referee serves; the agent's group may reach the socket, nothing else in the home.
chgrp gatesub "$home"
chmod 0710 "$home"
as referee "'$bin' --project '$ref' gate serve --referee '$home' --http 0 >'$ref/serve.out' 2>'$ref/serve.err' &"
i=0
while [ ! -S "$home/gate.sock" ] || [ ! -s "$ref/serve.out" ]; do
    i=$((i + 1))
    [ $i -lt 100 ] || fail "gate serve did not start: $(cat "$ref/serve.err")"
    sleep 0.1
done
chgrp gatesub "$home/gate.sock"
chmod 0660 "$home/gate.sock"
url=$(field http <"$ref/serve.out")

if as agent "cat '$home/identity.txt'" >/dev/null 2>&1; then fail "the agent can read the identity"; fi
if as agent "cat '$home/ledger.jsonl'" >/dev/null 2>&1; then fail "the agent can read the referee ledger"; fi

submit() {
    as agent "'$bin' --project '$agent' --format json gate submit --approved '$approved' --candidate triage-v2 --records '$work/candidate.jsonl' $*"
}
out=$(submit "--socket '$home/gate.sock'") || fail "socket submission failed: $out"
printf '%s' "$out" | grep -q '"code": "pass"' || fail "socket submission did not pass: $out"

# The token is handed to the agent out of band.
cp "$home/token" "$work/agent-token"
chown agent "$work/agent-token"
chmod 600 "$work/agent-token"
# A second, distinct candidate: the same run with other attributes.
sed '1s/}$/,"attrs":{"try":2}}/' "$work/candidate.jsonl" >"$work/candidate2.jsonl"
chmod 644 "$work/candidate2.jsonl"
out=$(as agent "'$bin' --project '$agent' --format json gate submit --approved '$approved' --candidate triage-v2 --records '$work/candidate2.jsonl' --url '$url' --token-file '$work/agent-token'") ||
    fail "HTTP submission failed: $out"
printf '%s' "$out" | grep -q '"used": 2' || fail "HTTP submission did not spend the second submission: $out"

gates=$(grep -c '"kind":"gate"' "$home/ledger.jsonl")
[ "$gates" = 2 ] || fail "expected 2 gate lines in the referee ledger, found $gates"
if command -v pkill >/dev/null 2>&1; then pkill -u referee -f 'gate serve' || true; fi
echo "gate two users passed: socket and HTTP submissions from user agent, 2 gate lines in the referee ledger"
