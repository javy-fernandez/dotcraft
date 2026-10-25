#!/bin/zsh
# Builds DotCraft in release mode and packages it (everything it needs is inside the binary).
# Options:
#   --universal  Apple Silicon + Intel macOS binary
#   --dmg        target/release/DotCraft-<version>-<arch>.dmg
#   --zip        target/release/DotCraft-<version>-macos-<arch>.zip (the .app)
#   --windows    Windows builds with cargo-xwin (ARM64 and x64, DirectX 12), zipped as
#                target/release/DotCraft-<version>-windows-<arch>.zip
#   --install    copies the app to /Applications
set -euo pipefail
cd "${0:A:h}/.."
source "$HOME/.cargo/env" 2>/dev/null || true

UNIVERSAL=0 DMG=0 ZIP=0 WINDOWS=0 INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=1 ;;
    --dmg) DMG=1 ;;
    --zip) ZIP=1 ;;
    --windows) WINDOWS=1 ;;
    --install) INSTALL=1 ;;
    *) echo "Unknown option: $arg" >&2; exit 1 ;;
  esac
done

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
if (( UNIVERSAL )); then
  cargo build --release -p rc-app --target aarch64-apple-darwin
  cargo build --release -p rc-app --target x86_64-apple-darwin
  lipo -create -output target/release/dotcraft-universal \
    target/aarch64-apple-darwin/release/dotcraft target/x86_64-apple-darwin/release/dotcraft
  BIN=target/release/dotcraft-universal ARCH=universal
else
  cargo build --release -p rc-app
  BIN=target/release/dotcraft ARCH=$(uname -m)
fi

APP=target/release/DotCraft.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/DotCraft"
cp assets/DotCraft.icns "$APP/Contents/Resources/DotCraft.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DotCraft</string>
  <key>CFBundleDisplayName</key><string>DotCraft</string>
  <key>CFBundleExecutable</key><string>DotCraft</string>
  <key>CFBundleIdentifier</key><string>app.dotcraft</string>
  <key>CFBundleIconFile</key><string>DotCraft</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key>
  <array>
    <string>en</string><string>es</string><string>de</string><string>fr</string>
    <string>ja</string><string>pl</string><string>pt-BR</string><string>zh-Hans</string>
  </array>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSRequiresAquaSystemAppearance</key><false/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>.NET Solution or Project</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>sln</string><string>slnx</string><string>slnf</string>
        <string>csproj</string><string>fsproj</string><string>vbproj</string>
        <string>props</string><string>targets</string>
      </array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Source Code</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>cs</string><string>csx</string><string>fs</string><string>vb</string>
        <string>razor</string><string>cshtml</string><string>xaml</string><string>axaml</string>
        <string>editorconfig</string><string>http</string><string>resx</string><string>config</string>
      </array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Text Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.text</string>
        <string>public.plain-text</string>
        <string>public.source-code</string>
        <string>public.script</string>
        <string>public.shell-script</string>
        <string>public.json</string>
        <string>public.xml</string>
        <string>public.yaml</string>
        <string>public.comma-separated-values-text</string>
        <string>net.daringfireball.markdown</string>
        <string>public.log</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Signing: a stable certificate keeps the Keychain from asking again after every build (the
# NuGet and Git credentials); without it, an ad hoc signature.
IDENTITY="${DOTCRAFT_SIGN_IDENTITY:-DotCraft}"
if security find-certificate -c "$IDENTITY" >/dev/null 2>&1 \
   && codesign --force --deep --identifier app.dotcraft --sign "$IDENTITY" "$APP" 2>/dev/null; then
  echo "Signed with the \"$IDENTITY\" certificate"
else
  codesign --force --deep --identifier app.dotcraft --sign - "$APP"
  echo "Warning: ad hoc signature (create the \"$IDENTITY\" certificate for a stable one)"
fi
echo "Generated $APP"

if (( DMG )); then
  DMG_FILE="target/release/DotCraft-${VERSION}-${ARCH}.dmg"
  STAGE=$(mktemp -d)
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG_FILE"
  hdiutil create -volname DotCraft -srcfolder "$STAGE" -format UDZO "$DMG_FILE" >/dev/null
  rm -rf "$STAGE"
  echo "Generated $DMG_FILE"
fi

if (( ZIP )); then
  ZIP_FILE="target/release/DotCraft-${VERSION}-macos-${ARCH}.zip"
  rm -f "$ZIP_FILE"
  ditto -c -k --keepParent "$APP" "$ZIP_FILE"
  echo "Generated $ZIP_FILE"
fi

if (( WINDOWS )); then
  for target in aarch64-pc-windows-msvc x86_64-pc-windows-msvc; do
    rustup target add "$target" >/dev/null
    cargo xwin build --release -p rc-app --target "$target"
    case "$target" in aarch64*) WARCH=arm64 ;; *) WARCH=x64 ;; esac
    STAGE=$(mktemp -d)
    mkdir "$STAGE/DotCraft"
    cp "target/$target/release/dotcraft.exe" "$STAGE/DotCraft/DotCraft.exe"
    cp LICENSE-MIT LICENSE-APACHE "$STAGE/DotCraft/"
    ZIP_FILE="$PWD/target/release/DotCraft-${VERSION}-windows-${WARCH}.zip"
    rm -f "$ZIP_FILE"
    (cd "$STAGE" && zip -qr "$ZIP_FILE" DotCraft)
    rm -rf "$STAGE"
    echo "Generated $ZIP_FILE"
  done
fi

if (( INSTALL )); then
  rm -rf /Applications/DotCraft.app
  cp -R "$APP" /Applications/
  echo "Installed at /Applications/DotCraft.app"
fi
