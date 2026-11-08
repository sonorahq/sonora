#!/usr/bin/env bash
set -euo pipefail

# Downloads one published asset and verifies its checksum before making it available to packaging.
tag=$1
asset=$2
destination=$3
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

gh release download "$tag" --repo "$GITHUB_REPOSITORY" \
  --pattern SHA256SUMS --pattern "$asset" --dir "$temporary"
expected=$(awk -v asset="$asset" '$2 == asset { print $1 }' "$temporary/SHA256SUMS")
if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
  echo "missing or ambiguous checksum for $asset" >&2
  exit 1
fi
if command -v sha256sum >/dev/null; then
  actual=$(sha256sum "$temporary/$asset" | cut -d ' ' -f 1)
else
  actual=$(shasum -a 256 "$temporary/$asset" | cut -d ' ' -f 1)
fi
if [[ "$actual" != "$expected" ]]; then
  echo "checksum mismatch for $asset" >&2
  exit 1
fi
mkdir -p "$(dirname "$destination")"
cp "$temporary/$asset" "$destination"
