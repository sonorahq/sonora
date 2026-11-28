#!/bin/bash
set -e

# Set working directory to the script's directory
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
pushd "$SCRIPT_DIR" > /dev/null
trap 'popd > /dev/null 2>&1' EXIT

# Run source generation
./generate-sources.sh

# Check if running inside a Dev Container or Docker container
if [ -n "$REMOTE_CONTAINERS" ] || [ -f "/.dockerenv" ] || [ -f "/run/.containerenv" ]; then
  echo "WARNING: Container environment detected, building a flatpak inside a container will likely fail..."
fi

# Check for flatpak-builder
if ! command -v flatpak-builder >/dev/null 2>&1; then
    echo "=========================================================================="
    echo " ERROR: 'flatpak-builder' is not installed."
    echo ""
    echo " To install it, run the command for your distribution:"
    echo "   Fedora:        sudo dnf install flatpak-builder"
    echo "   Ubuntu/Debian: sudo apt install flatpak-builder"
    echo "   Arch Linux:    sudo pacman -S flatpak-builder"
    echo "=========================================================================="
    exit 1
fi

# Check for required Flatpak runtimes & SDKs
echo "==> Checking required Flatpak SDKs..."
REQUIRED_REFS="
org.gnome.Platform//50
org.gnome.Sdk//50
"

MISSING_REFS=""
for ref in $REQUIRED_REFS; do
    if ! flatpak info "$ref" >/dev/null 2>&1; then
        MISSING_REFS="$MISSING_REFS $ref"
    fi
done

# If dependencies are missing, guide the user without forcing system modifications
if [ -n "$MISSING_REFS" ]; then
    echo "=========================================================================="
    echo " ERROR: Missing required Flatpak runtimes/SDKs:"
    for ref in $MISSING_REFS; do
        echo "   - $ref"
    done
    echo ""
    echo " To INSTALL them, run:"

    # Check if flathub remote is configured anywhere on the system
    if ! flatpak remotes | grep -q "flathub"; then
        echo "   flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo"
    fi

    echo "   flatpak install $MISSING_REFS"
    echo ""
    echo " To REMOVE them later when done with development, run:"
    echo "   flatpak uninstall $MISSING_REFS"
    echo "=========================================================================="
    exit 1
fi

echo "  All required Flatpak dependencies are installed."

# Run flatpak-builder
echo "==> Building Flatpak..."
cd ../..
flatpak-builder --force-clean --user --disable-cache --repo packaging/flatpak/flatpak-repo packaging/flatpak/flatpak packaging/flatpak/flatpak-builder.yaml
cd packaging/flatpak
echo "==> Build complete!"

echo "==> Packaging into .flatpak"

flatpak build-bundle flatpak-repo sonora.flatpak io.github.nolight132.sonora

echo "==> Finished"