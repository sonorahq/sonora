#!/bin/bash
set -e

# Set working directory to the script's directory
pushd "$(dirname "$(readlink -f "$0")")" > /dev/null
trap 'popd > /dev/null 2>&1' EXIT

echo "Current directory inside script: $(pwd)"

# Check if required tools are installed
if ! command -v flatpak-cargo-generator >/dev/null 2>&1; then
    echo "=========================================================================="
    echo " ERROR: 'flatpak-cargo-generator' is not installed or not in PATH."
    echo ""
    echo " To install it, run:"
    echo "   pipx install flatpak-cargo-generator"
    echo ""
    echo " Ensure pipx binaries are available in your PATH:"
    echo "   export PATH=\"\$PATH:\$HOME/.local/bin\""
    echo "=========================================================================="
    exit 1
fi

# Generate cargo-sources
echo "==> Generating cargo-sources.json..."
# cargo-generator.py patches how the generator inherits workspace dependencies, so it runs on the
# interpreter the installed generator runs on.
python="$(sed -n '1s/^#!//p' "$(command -v flatpak-cargo-generator)")"
$python cargo-generator.py -o cargo-sources.json ../Cargo.lock

echo "==> Preparation complete!"
