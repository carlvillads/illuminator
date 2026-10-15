#!/usr/bin/env bash
# Prints the CHANGELOG.md notes of one version for a GitHub release.
#
# Usage: .github/scripts/release-notes.sh [VERSION]
# VERSION defaults to the version in Cargo.toml and MUST match it.
#
# Fails unless Cargo.toml, CHANGELOG.md and the AppStream metainfo agree.
# The changelog needs a "## [VERSION] - YYYY-MM-DD" section with notes, and the metainfo needs a
# <release version="VERSION" date="YYYY-MM-DD"> with the same date.
set -euo pipefail
cd "$(dirname "$0")/../.."

metainfo=crates/illuminator-app/packaging/linux/io.github.carlvillads.illuminator.metainfo.xml

fail() {
    echo "error: $*" >&2
    exit 1
}

cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
version=${1:-$cargo_version}
if [ "$version" != "$cargo_version" ]; then
    fail "version $version doesn't match version $cargo_version in Cargo.toml"
fi

heading=$(awk -v version="$version" 'index($0, "## [" version "] - ") == 1 { print; exit }' CHANGELOG.md)
if [ -z "$heading" ]; then
    fail "CHANGELOG.md has no '## [$version] - YYYY-MM-DD' section"
fi
date=${heading##* - }
if ! grep -qF "<release version=\"$version\" date=\"$date\"" "$metainfo"; then
    fail "$metainfo has no <release version=\"$version\" date=\"$date\">"
fi

notes=$(awk -v heading="$heading" '
    $0 == heading { found = 1; next }
    !found { next }
    /^## / || /^\[.*\]: / { exit }
    NF == 0 { if (printed) blank++; next }
    { for (; blank > 0; blank--) print ""; print; printed = 1 }
' CHANGELOG.md)
if [ -z "$notes" ]; then
    fail "the $version section of CHANGELOG.md is empty"
fi
printf '%s\n' "$notes"
