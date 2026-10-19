#!/bin/sh
# Builds dist/Satsuma.app: release binary, Info.plist (menu-bar agent), icon, ad-hoc signature.
set -eu

cd "$(dirname "$0")/.."
# Keep build-machine paths (home folder, cargo registry, checkout) out of the binary.
# When several prefixes match, the last one wins.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~ --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$PWD=."
cargo build --release -p satsuma-app

APP=dist/Satsuma.app
BIN=target/release/satsuma
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Satsuma"

# Icon: render each size with the app itself, then pack with iconutil.
ICONSET=$(mktemp -d)/Satsuma.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  "$BIN" render-icon --size "$s" --out "$ICONSET/icon_${s}x${s}.png"
  "$BIN" render-icon --size $((s * 2)) --out "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Satsuma.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Satsuma</string>
  <key>CFBundleDisplayName</key><string>Satsuma</string>
  <key>CFBundleIdentifier</key><string>app.satsuma.Satsuma</string>
  <key>CFBundleExecutable</key><string>Satsuma</string>
  <key>CFBundleIconFile</key><string>Satsuma</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null
echo "Built $APP"
echo "Install: cp -R $APP /Applications/   (or open it from dist/)"
