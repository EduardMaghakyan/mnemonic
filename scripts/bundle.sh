#!/usr/bin/env bash
# Build the .app and .dmg for Apple Silicon.
# The CLI is bundled inside the .app via Tauri's externalBin sidecar mechanism,
# which expects the source binary to be named with the target triple suffix.
#
# If .env is present at the repo root, it is sourced so cargo tauri build can
# sign + notarize. Required vars:
#   APPLE_SIGNING_IDENTITY
#   APPLE_ID
#   APPLE_PASSWORD       (app-specific password)
#   APPLE_TEAM_ID
# When any of these are missing the build proceeds unsigned.
set -euo pipefail

TARGET=aarch64-apple-darwin
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -f .env ]; then
  echo "==> Sourcing .env"
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] \
   && [ -n "${APPLE_ID:-}" ] \
   && [ -n "${APPLE_PASSWORD:-}" ] \
   && [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "==> Signing + notarizing as ${APPLE_SIGNING_IDENTITY}"
  SIGNED=1
else
  echo "==> .env incomplete or missing — building UNSIGNED"
  SIGNED=0
fi

echo "==> Building mnemonic CLI (release, $TARGET)"
cargo build -p mnemonic-cli --release --target "$TARGET"

echo "==> Staging CLI for Tauri externalBin"
mkdir -p app/binaries
cp "target/$TARGET/release/mnemonic" "app/binaries/mnemonic-$TARGET"

echo "==> Tauri build (.app + .dmg)"
cargo tauri build --target "$TARGET"

APP="$ROOT/target/$TARGET/release/bundle/macos/Mnemonic.app"
DMG="$(find "$ROOT/target/$TARGET/release/bundle/dmg" -maxdepth 1 -name "Mnemonic_*_aarch64.dmg" | head -1)"
echo
echo "Built:"
echo "  $APP"
echo "  $DMG"

if [ "$SIGNED" = "1" ]; then
  echo
  echo "==> Verifying signature + Gatekeeper"
  codesign --verify --deep --strict --verbose=2 "$APP" || true
  spctl --assess --type execute --verbose "$APP" || true
fi

echo
echo "SHA256:"
shasum -a 256 "$DMG"
