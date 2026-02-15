#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ICONS_DIR="$PROJECT_DIR/src-tauri/icons"
SRC="$ICONS_DIR/icon.png"
TMP_DIR=$(mktemp -d)

if [ ! -f "$SRC" ]; then
  echo "Error: $SRC not found"
  exit 1
fi

echo "==> Generating icons from $SRC"

# PNG sizes
node -e "
const sharp = require('sharp');
const path = require('path');
(async () => {
  const src = '$SRC';
  const dir = '$ICONS_DIR';
  const sizes = [[32,'32x32.png'],[64,'64x64.png'],[128,'128x128.png'],[256,'128x128@2x.png']];
  for (const [size, name] of sizes) {
    await sharp(src).resize(size,size).ensureAlpha().png().toFile(path.join(dir, name));
  }
  // Store logos
  const store = [[30,'Square30x30Logo.png'],[44,'Square44x44Logo.png'],[71,'Square71x71Logo.png'],[89,'Square89x89Logo.png'],[107,'Square107x107Logo.png'],[142,'Square142x142Logo.png'],[150,'Square150x150Logo.png'],[284,'Square284x284Logo.png'],[310,'Square310x310Logo.png'],[50,'StoreLogo.png']];
  for (const [size, name] of store) {
    await sharp(src).resize(size,size).ensureAlpha().png().toFile(path.join(dir, name));
  }
  // Favicon
  await sharp(src).resize(32,32).ensureAlpha().png().toFile(path.join('$PROJECT_DIR', 'public/favicon.png'));
  console.log('  PNGs + favicon done');
})();
"

# .icns
ICONSET="$TMP_DIR/icon.iconset"
mkdir -p "$ICONSET"
node -e "
const sharp = require('sharp');
const path = require('path');
(async () => {
  const src = '$SRC';
  const dir = '$ICONSET';
  const sizes = [[16,'icon_16x16.png'],[32,'icon_16x16@2x.png'],[32,'icon_32x32.png'],[64,'icon_32x32@2x.png'],[128,'icon_128x128.png'],[256,'icon_128x128@2x.png'],[256,'icon_256x256.png'],[512,'icon_256x256@2x.png'],[512,'icon_512x512.png'],[1024,'icon_512x512@2x.png']];
  for (const [size, name] of sizes) {
    await sharp(src).resize(size,size).ensureAlpha().png().toFile(path.join(dir, name));
  }
})();
"
iconutil -c icns "$ICONSET" -o "$ICONS_DIR/icon.icns"
echo "  .icns done"

# .ico
node -e "
const sharp = require('sharp');
const fs = require('fs');
(async () => {
  const src = '$SRC';
  const sizes = [16,32,48,64,128,256];
  const bufs = [];
  for (const s of sizes) bufs.push(await sharp(src).resize(s,s).ensureAlpha().png().toBuffer());
  const hdr = Buffer.alloc(6); hdr.writeUInt16LE(0,0); hdr.writeUInt16LE(1,2); hdr.writeUInt16LE(sizes.length,4);
  const dir = Buffer.alloc(16*sizes.length);
  let off = 6 + 16*sizes.length;
  for (let i=0;i<sizes.length;i++) {
    const o=i*16, s=sizes[i];
    dir.writeUInt8(s>=256?0:s,o); dir.writeUInt8(s>=256?0:s,o+1);
    dir.writeUInt8(0,o+2); dir.writeUInt8(0,o+3);
    dir.writeUInt16LE(1,o+4); dir.writeUInt16LE(32,o+6);
    dir.writeUInt32LE(bufs[i].length,o+8); dir.writeUInt32LE(off,o+12);
    off+=bufs[i].length;
  }
  fs.writeFileSync('$ICONS_DIR/icon.ico', Buffer.concat([hdr,dir,...bufs]));
  console.log('  .ico done');
})();
"

# Assets.car (macOS Tahoe)
ICON_FILE="$ICONS_DIR/AppIcon.icon"
if [ -d "$ICON_FILE" ]; then
  ASSETS_OUT="$TMP_DIR/assets"
  mkdir -p "$ASSETS_OUT"
  xcrun actool "$ICON_FILE" --compile "$ASSETS_OUT" \
    --output-format human-readable-text --notices --warnings --errors \
    --output-partial-info-plist "$ASSETS_OUT/Info.plist" \
    --app-icon AppIcon --include-all-app-icons \
    --enable-on-demand-resources NO \
    --target-device mac \
    --minimum-deployment-target 13.0 \
    --platform macosx
  cp "$ASSETS_OUT/Assets.car" "$ICONS_DIR/Assets.car"
  echo "  Assets.car done"
else
  echo "  Skipping Assets.car (AppIcon.icon not found)"
fi

rm -rf "$TMP_DIR"
echo "==> All icons generated"
