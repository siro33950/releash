#!/bin/bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
WORK=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/releash-release.XXXXXX")
KEYCHAIN="$WORK/signing.keychain-db"
KEYCHAIN_PASSWORD=$(openssl rand -hex 32)
cleanup() {
  set +e
  if [ -f "$WORK/keychains.json" ]; then
    python3 - "$WORK/keychains.json" <<'PYKEYS'
import json, subprocess, sys
with open(sys.argv[1]) as file:
    subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', *json.load(file)], check=True)
PYKEYS
  fi
  security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

umask 077
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$WORK/certificate.p12"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$WORK/certificate.p12" -k "$KEYCHAIN" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
rm "$WORK/certificate.p12"
python3 - "$KEYCHAIN" "$WORK/keychains.json" <<'PYKEYS'
import json, shlex, subprocess, sys
keychains = shlex.split(subprocess.check_output(['security', 'list-keychains', '-d', 'user'], text=True))
with open(sys.argv[2], 'w') as file:
    json.dump(keychains, file)
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', sys.argv[1], *keychains], check=True)
PYKEYS

./nx run macos:generate
./nx run macos:helpers -c release
for binary in "$ROOT/clients/macos/.build/Helpers/Release/releashd" "$ROOT/clients/macos/.build/Helpers/Release/releash"; do
  lipo -verify_arch arm64 "$binary"
  lipo -verify_arch x86_64 "$binary"
  LINKED_LIBRARIES=$(otool -L "$binary")
  if grep -Eq '/(opt/homebrew|usr/local)/' <<< "$LINKED_LIBRARIES"; then
    echo "::error::Helper depends on a local package manager library: $binary"
    exit 1
  fi
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$KEYCHAIN" --options runtime --timestamp "$binary"
done
./nx run macos:file-viewer
cd clients/macos
swift run --package-path BuildTools xcodegen generate
xcodebuild -resolvePackageDependencies -project Releash.xcodeproj -scheme Releash \
  -clonedSourcePackagesDirPath .build/SourcePackages -onlyUsePackageVersionsFromResolvedFile
xcodebuild -project Releash.xcodeproj -scheme Releash -configuration Release \
  -destination 'generic/platform=macOS' -archivePath "$WORK/Releash.xcarchive" \
  -clonedSourcePackagesDirPath .build/SourcePackages -disableAutomaticPackageResolution \
  -skipPackagePluginValidation 'ARCHS=arm64 x86_64' ONLY_ACTIVE_ARCH=NO \
  CODE_SIGN_STYLE=Manual "CODE_SIGN_IDENTITY=$APPLE_SIGNING_IDENTITY" \
  "DEVELOPMENT_TEAM=$APPLE_TEAM_ID" "OTHER_CODE_SIGN_FLAGS=--keychain $KEYCHAIN --timestamp" \
  archive

APP="$WORK/Releash.xcarchive/Products/Applications/Releash Preview.app"
VERSION=$(python3 - "$ROOT" "$APP" <<'PY'
import json, plistlib, sys, tomllib
from pathlib import Path
root, app = map(Path, sys.argv[1:])
version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
with (app / 'Contents/Info.plist').open('rb') as file:
    info = plistlib.load(file)
assert info['CFBundleShortVersionString'] == version, 'Swift/Cargo version mismatch'
assert json.loads((root / 'clients/desktop/package.json').read_text())['version'] == version, 'Tauri/Cargo version mismatch'
assert info['CFBundleIdentifier'] == 'com.releash.macos', 'Unexpected bundle ID'
print(version)
PY
)
lipo -verify_arch arm64 "$APP/Contents/MacOS/Releash Preview"
lipo -verify_arch x86_64 "$APP/Contents/MacOS/Releash Preview"
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$KEYCHAIN" --options runtime --timestamp "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

ditto -c -k --keepParent "$APP" "$WORK/notarize.zip"
mkdir -p .build/Distribution
notary() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if xcrun notarytool "$@" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" --output-format json > .build/Distribution/notary.json; then
      return 0
    fi
    echo "::warning::notarytool $1 failed (attempt $attempt)"
    sleep $((attempt * 30))
  done
  return 1
}
notary submit "$WORK/notarize.zip"
SUBMISSION_ID=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["id"])' .build/Distribution/notary.json)
notary wait "$SUBMISSION_ID" --timeout 30m
python3 - .build/Distribution/notary.json <<'PY'
import json, sys
with open(sys.argv[1]) as file:
    result = json.load(file)
print('Notarization:', result['id'], result['status'])
if result['status'] != 'Accepted':
    raise SystemExit('Notarization was not accepted')
PY
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose=2 "$APP"
mkdir -p .build/Distribution
ditto -c -k --keepParent "$APP" ".build/Distribution/Releash_Preview_${VERSION}_universal.zip"
