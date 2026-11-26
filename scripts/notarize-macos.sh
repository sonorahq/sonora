#!/usr/bin/env bash
set -euo pipefail

# Sends one zip, disk image or package to Apple's notary service, waits for the verdict and prints
# Apple's log when it is anything but Accepted. Reads the App Store Connect API key from
# APPLE_API_KEY_PATH, APPLE_API_KEY_ID and APPLE_API_ISSUER.
file=$1

result=$(xcrun notarytool submit "$file" \
  --key "$APPLE_API_KEY_PATH" \
  --key-id "$APPLE_API_KEY_ID" \
  --issuer "$APPLE_API_ISSUER" \
  --wait \
  --timeout 1h \
  --output-format json)
id=$(jq -r .id <<<"$result")
status=$(jq -r .status <<<"$result")
echo "notarization $id of $(basename "$file"): $status"

if [ "$status" != Accepted ]; then
  xcrun notarytool log "$id" \
    --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER"
  exit 1
fi
