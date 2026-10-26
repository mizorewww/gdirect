#!/bin/bash
set -euo pipefail

MACOS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RELEASE_JSON="$MACOS_DIR/Resources/CLIRelease.json"
FINAL_APP="$MACOS_DIR/dist/GDirect.app"
LOCAL_CLI=
CLI_SUPPLIED=0

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  printf 'Usage: %s [--cli /absolute/path/to/gdirect]\n' "$0" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cli)
      [[ $# -eq 2 ]] || usage
      LOCAL_CLI="$2"
      CLI_SUPPLIED=1
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
  arm64|x86_64) ;;
  *) fail "Unsupported architecture: $HOST_ARCH (supported: arm64, x86_64)" ;;
esac

if [[ "$CLI_SUPPLIED" -eq 1 ]]; then
  [[ -n "$LOCAL_CLI" && "$LOCAL_CLI" == /* ]] || fail "--cli requires a nonempty absolute path"
fi
[[ -f "$RELEASE_JSON" ]] || fail "Missing release pins: $RELEASE_JSON"

release_info=$(python3 - "$RELEASE_JSON" "$HOST_ARCH" <<'PY'
import json
import re
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    release = json.load(stream)
version = release["version"]
asset = release["archives"][sys.argv[2]]["asset"]
sha256 = release["archives"][sys.argv[2]]["sha256"]
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
    raise ValueError("Pinned CLI version must be a semantic version")
if not re.fullmatch(r"[A-Za-z0-9._-]+", asset) or not asset.endswith(".tar.gz"):
    raise ValueError("Pinned archive asset has an unsafe or unsupported name")
if not re.fullmatch(r"[0-9a-f]{64}", sha256):
    raise ValueError("Pinned archive SHA256 must be 64 lowercase hexadecimal characters")
release_url = release["release_url"]
if not release_url.startswith("https://"):
    raise ValueError("Release URL must use HTTPS")
print("\t".join((version, asset, sha256, release_url)))
PY
) || fail "Could not read CLI release pins"
IFS=$'\t' read -r CLI_VERSION ASSET EXPECTED_SHA RELEASE_URL <<< "$release_info"

SCRATCH_PARENT=${GDIRECT_BUILD_SCRATCH_PARENT:-${TMPDIR:-/tmp}}
[[ "$SCRATCH_PARENT" = /* ]] || fail "Build scratch parent must be an absolute path"
[[ -d "$SCRATCH_PARENT" ]] || fail "Build scratch parent does not exist: $SCRATCH_PARENT"
WORK_DIR=$(mktemp -d "$SCRATCH_PARENT/gdirect-app.XXXXXX")
cleanup() {
  [[ -n "${WORK_DIR:-}" && -d "$WORK_DIR" ]] && rm -rf "$WORK_DIR"
  [[ -n "${DIST_STAGING:-}" && -d "$DIST_STAGING" ]] && rm -rf "$DIST_STAGING"
  if [[ -n "${PREVIOUS_APP:-}" && -d "$PREVIOUS_APP" ]]; then
    if [[ -e "$FINAL_APP" ]]; then
      rm -rf "$PREVIOUS_APP"
    else
      mv "$PREVIOUS_APP" "$FINAL_APP"
    fi
  fi
}
trap cleanup EXIT
SWIFT_SCRATCH="$WORK_DIR/swift"

printf 'Running native GUI tests...\n'
if ! swift test --package-path "$MACOS_DIR" --scratch-path "$SWIFT_SCRATCH"; then
  DEVELOPER_DIR=$(xcode-select -p 2>/dev/null || true)
  TESTING_MACROS="$DEVELOPER_DIR/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
  [[ -f "$TESTING_MACROS" ]] || fail "swift test failed and this toolchain has no TestingMacros fallback"
  printf 'Retrying tests with the active toolchain TestingMacros library...\n'
  swift test --package-path "$MACOS_DIR" --scratch-path "$SWIFT_SCRATCH" \
    -Xswiftc -load-plugin-library -Xswiftc "$TESTING_MACROS"
fi

printf 'Building GDirectApp (release)...\n'
swift build -c release --package-path "$MACOS_DIR" --scratch-path "$SWIFT_SCRATCH"
GUI_BINARY="$SWIFT_SCRATCH/release/GDirectApp"
[[ -x "$GUI_BINARY" ]] || fail "Release GUI executable is missing: $GUI_BINARY"

require_architecture() {
  local executable="$1"
  local label="$2"
  local architectures
  architectures=$(lipo -archs "$executable" 2>&1) || fail "$label is not a readable Mach-O executable: $architectures"
  local arch
  for arch in $architectures; do
    [[ "$arch" == "$HOST_ARCH" ]] && return 0
  done
  fail "$label architecture ($architectures) does not match host $HOST_ARCH"
}

validate_cli() {
  local executable="$1"
  local output
  output=$("$executable" --version 2>&1) || fail "CLI --version failed: $output"
  [[ "$output" == "gdirect $CLI_VERSION" ]] || \
    fail "CLI version mismatch: expected gdirect $CLI_VERSION, got $output"
  require_architecture "$executable" "CLI"
}

if [[ -n "$LOCAL_CLI" ]]; then
  [[ -f "$LOCAL_CLI" && -x "$LOCAL_CLI" ]] || fail "Local CLI is not an executable file: $LOCAL_CLI"
  CLI_SOURCE="$LOCAL_CLI"
else
  ARCHIVE="$WORK_DIR/$ASSET"
  printf 'Downloading pinned gdirect %s for %s...\n' "$CLI_VERSION" "$HOST_ARCH"
  curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
    --output "$ARCHIVE" "$RELEASE_URL/$ASSET"
  read -r ACTUAL_SHA _ < <(shasum -a 256 "$ARCHIVE")
  [[ "$ACTUAL_SHA" == "$EXPECTED_SHA" ]] || \
    fail "Checksum mismatch for $ASSET: expected $EXPECTED_SHA, got $ACTUAL_SHA"
  mkdir -p "$WORK_DIR/extracted"
  tar -xzf "$ARCHIVE" -C "$WORK_DIR/extracted"
  CLI_DIR="$WORK_DIR/extracted/${ASSET%.tar.gz}"
  CLI_SOURCE="$CLI_DIR/gdirect"
  [[ -x "$CLI_SOURCE" ]] || fail "Verified archive does not contain an executable gdirect"
fi

validate_cli "$CLI_SOURCE"
require_architecture "$GUI_BINARY" "GDirectApp"

STAGING_APP="$WORK_DIR/GDirect.app"
mkdir -p "$STAGING_APP/Contents/MacOS" "$STAGING_APP/Contents/Resources"
cp "$MACOS_DIR/Resources/Info.plist" "$STAGING_APP/Contents/Info.plist"
cp "$GUI_BINARY" "$STAGING_APP/Contents/MacOS/GDirectApp"
cp "$CLI_SOURCE" "$STAGING_APP/Contents/MacOS/gdirect"
chmod 755 "$STAGING_APP/Contents/MacOS/GDirectApp" "$STAGING_APP/Contents/MacOS/gdirect"
xattr -rc "$STAGING_APP" >/dev/null 2>&1 || true

SIGNING_IDENTITY=${CODESIGN_IDENTITY:--}
printf 'Signing bundle with identity: %s\n' "$SIGNING_IDENTITY"
codesign --force --options runtime --sign "$SIGNING_IDENTITY" "$STAGING_APP/Contents/MacOS/gdirect"
codesign --force --options runtime --sign "$SIGNING_IDENTITY" "$STAGING_APP/Contents/MacOS/GDirectApp"
codesign --force --options runtime --sign "$SIGNING_IDENTITY" "$STAGING_APP"
python3 "$MACOS_DIR/Scripts/verify-app.py" "$STAGING_APP" --version "$CLI_VERSION" --arch "$HOST_ARCH"

mkdir -p "$MACOS_DIR/dist"
DIST_STAGING=$(mktemp -d "$MACOS_DIR/dist/.GDirect.app.tmp.XXXXXX")
ditto --norsrc "$STAGING_APP" "$DIST_STAGING"
FINAL_VERIFIED=
for attempt in 1 2 3; do
  xattr -rc "$DIST_STAGING" >/dev/null 2>&1 || true
  if python3 "$MACOS_DIR/Scripts/verify-app.py" "$DIST_STAGING" --version "$CLI_VERSION" --arch "$HOST_ARCH"; then
    FINAL_VERIFIED=1
    break
  fi
  [[ "$attempt" -lt 3 ]] && sleep 1
done
[[ -n "$FINAL_VERIFIED" ]] || fail "Final APP verification failed after clearing copied metadata"

PREVIOUS_APP=
if [[ -e "$FINAL_APP" ]]; then
  PREVIOUS_APP=$(mktemp -d "$MACOS_DIR/dist/.GDirect.app.previous.XXXXXX")
  rmdir "$PREVIOUS_APP"
  mv "$FINAL_APP" "$PREVIOUS_APP"
fi
mv "$DIST_STAGING" "$FINAL_APP"
DIST_STAGING=
if [[ -n "$PREVIOUS_APP" ]]; then
  rm -rf "$PREVIOUS_APP"
  PREVIOUS_APP=
fi
printf 'Built %s\n' "$FINAL_APP"
