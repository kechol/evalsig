#!/bin/sh
# Refuse a release tag whose version has no dated section in CHANGELOG.md.
# Usage: scripts/check_changelog.sh <tag> [changelog]   (for example v0.1.0 or app/v0.1.0)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tag=${1:?usage: check_changelog.sh <tag> [changelog]}
changelog=${2:-$root/CHANGELOG.md}
version=${tag##*/}
version=${version#v}
heading=$(awk -v h="## [$version]" 'index($0, h) == 1 { print; exit }' "$changelog")
if [ -z "$heading" ]; then
    echo "CHANGELOG.md has no section for $version; add one before tagging $tag" >&2
    exit 1
fi
case $heading in
*Unreleased*)
    echo "the CHANGELOG.md section for $version is still marked Unreleased; date it before tagging $tag" >&2
    exit 1
    ;;
esac
echo "CHANGELOG.md has a section for $version"
