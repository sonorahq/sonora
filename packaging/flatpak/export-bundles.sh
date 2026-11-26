#!/bin/bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <ostree repo> <tag> <out dir>" >&2
  exit 1
fi

REPO="$1"
TAG="$2"
OUT="$3"
APP=io.github.nolight132.sonora
BRANCH=stable
RUNTIME_REPO=https://flathub.org/repo/flathub.flatpakrepo

SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
FLATPAKREPO="$SCRIPT_DIR/pages/sonora.flatpakrepo"

URL="$(sed -n 's/^Url=//p' "$FLATPAKREPO")"
KEYRING="$(mktemp)"
trap 'rm -f "$KEYRING"' EXIT
sed -n 's/^GPGKey=//p' "$FLATPAKREPO" | base64 -d > "$KEYRING"

mkdir -p "$OUT" "$REPO/tmp" "$REPO/refs/remotes" "$REPO/refs/mirrors"

ARCHES="$(ostree refs --repo="$REPO" | sed -n "s#^app/$APP/\([^/]*\)/$BRANCH\$#\1#p")"
if [ -z "$ARCHES" ]; then
  echo "no $APP refs in $REPO" >&2
  exit 1
fi

for arch in $ARCHES; do
  bundle="$OUT/sonora-$TAG-$arch.flatpak"
  echo "==> $bundle"
  flatpak build-bundle \
    --arch="$arch" \
    --repo-url="$URL" \
    --runtime-repo="$RUNTIME_REPO" \
    --gpg-keys="$KEYRING" \
    "$REPO" "$bundle" "$APP" "$BRANCH"
done
