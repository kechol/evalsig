#!/bin/sh
# Build evalsig in a Rust container and run scripts/gate_two_users.sh there, so the
# referee and the agent are separate OS users. Crates come from the host's registry
# (run `cargo fetch --locked` first); the container needs no network.
# Usage: scripts/gate_serve_docker.sh [image]   (default rust:1.95-slim)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
image=${1:-rust:1.95-slim}
registry=${CARGO_HOME:-$HOME/.cargo}/registry
docker run --rm \
    -v "$root:/src:ro" \
    -v "$registry:/usr/local/cargo/registry:ro" \
    -v evalsig-gate-target:/target \
    "$image" sh -c '
        set -eu
        cp -Rp /src /work && cd /work && rm -f rust-toolchain.toml
        cargo build --offline --locked -p evalsig-cli --target-dir /target >/dev/null 2>&1 ||
            cargo build --offline --locked -p evalsig-cli --target-dir /target
        sh scripts/gate_two_users.sh /target/debug/evalsig
    '
