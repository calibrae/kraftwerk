#!/usr/bin/env bash
#
# Bundle libvirt and its transitive dylib deps into a built Kraftwerk.app
# so the app runs on macs that don't have `brew install libvirt`.
#
# Run AFTER `tauri build` has produced a signed .app. This script:
#   1. Uses dylibbundler to copy every non-system dylib into
#      Contents/Frameworks/ and rewrite install_names to @executable_path-
#      relative paths.
#   2. Re-codesigns each embedded dylib (codesign breaks when we rewrite
#      load commands).
#   3. Re-codesigns the main binary with the project's hardened-runtime
#      entitlements.
#   4. Re-codesigns the .app bundle as a whole.
#
# The DMG and notarization step that Tauri ran are now invalidated —
# the caller must re-package + re-notarize. release.yml does both.
#
# Required env: APPLE_SIGNING_IDENTITY  (e.g. "Developer ID Application: ...")
# Optional env: ENTITLEMENTS_PATH       (defaults to src-tauri/entitlements.plist)
#
# Usage: scripts/bundle_macos_dylibs.sh <path-to-Kraftwerk.app>

set -euo pipefail

APP_PATH="${1:?usage: $0 <Kraftwerk.app>}"
APP_PATH="${APP_PATH%/}"
BIN_PATH="$APP_PATH/Contents/MacOS/kraftwerk"
FRAMEWORKS_DIR="$APP_PATH/Contents/Frameworks"

: "${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY must be set}"
ENTITLEMENTS_PATH="${ENTITLEMENTS_PATH:-$(dirname "$0")/../src-tauri/entitlements.plist}"

if [[ ! -x "$BIN_PATH" ]]; then
  echo "error: binary not found at $BIN_PATH" >&2
  exit 1
fi

if ! command -v dylibbundler >/dev/null 2>&1; then
  echo "error: dylibbundler not installed (brew install dylibbundler)" >&2
  exit 1
fi

echo "==> bundling dylibs into $FRAMEWORKS_DIR"
mkdir -p "$FRAMEWORKS_DIR"

# -od  overwrite output dir
# -b   bundle frameworks too (we don't actually have any, but safe)
# -ns  don't auto-strip (tauri build already produced release artifacts)
# -p   install_name prefix for rewrites
dylibbundler \
  -od \
  -b \
  -ns \
  -x "$BIN_PATH" \
  -d "$FRAMEWORKS_DIR" \
  -p '@executable_path/../Frameworks/' \
  >/tmp/dylibbundler.log 2>&1 || {
    echo "dylibbundler failed; tail of log:" >&2
    tail -40 /tmp/dylibbundler.log >&2
    exit 1
  }

echo "==> bundled $(ls "$FRAMEWORKS_DIR" | wc -l | tr -d ' ') libs ($(du -sh "$FRAMEWORKS_DIR" | cut -f1))"

echo "==> re-signing embedded dylibs"
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  codesign --force --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$dylib" 2>&1 \
    | grep -v "replacing existing signature" || true
done

echo "==> re-signing binary with entitlements"
codesign \
  --force \
  --options runtime \
  --entitlements "$ENTITLEMENTS_PATH" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "$BIN_PATH"

echo "==> re-signing app bundle"
codesign \
  --force \
  --options runtime \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "$APP_PATH"

echo "==> verifying"
codesign --verify --deep --strict --verbose=2 "$APP_PATH" 2>&1 | tail -5

echo "==> done. App is self-contained; users no longer need brew install libvirt."
