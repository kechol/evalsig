#!/bin/sh
# Refuse a release tag whose version has no section in CHANGELOG.md.
# Usage: scripts/check_changelog.sh <tag>   (for example v0.1.0)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tag=${1:?usage: check_changelog.sh <tag>}
version=${tag#v}
if grep -Eq "^## \[$(printf '%s' "$version" | sed 's/[.]/\\./g')\]( |$)" "$root/CHANGELOG.md"; then
    echo "CHANGELOG.md has a section for $version"
else
    echo "CHANGELOG.md has no section for $version; add one before tagging $tag" >&2
    exit 1
fi
