#!/bin/bash
# Packs Sonora's licence, the third-party notices and the font and icon licences into the
# tarball every release and nightly publishes beside its binaries.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <archive.tar.gz>" >&2
  exit 1
fi

ARCHIVE="$(readlink -f "$1")"
ROOT="$(dirname "$(dirname "$(readlink -f "$0")")")"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cd "$ROOT"
cp COPYING "$STAGE/LICENSE"
cp THIRD-PARTY.md "$STAGE/THIRD-PARTY.md"
cp assets/fonts/LICENSE.txt "$STAGE/LICENSE.Inter"
mkdir -p "$STAGE/icons"
cp assets/icons/LICENSE "$STAGE/icons/LICENSE"
for licence in assets/icons/*/LICENSE; do
  pack="$(basename "$(dirname "$licence")")"
  cp "$licence" "$STAGE/icons/LICENSE.$pack"
done
tar -czf "$ARCHIVE" -C "$STAGE" .
