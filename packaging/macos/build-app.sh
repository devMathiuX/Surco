#!/usr/bin/env bash
# Arma Surco.app (binario universal Intel + Apple Silicon) y un .dmg.
# Uso: packaging/macos/build-app.sh <versión> [carpeta de salida]
#
# Espera target/x86_64-apple-darwin/release/surco y
# target/aarch64-apple-darwin/release/surco ya compilados.
# La app va firmada "ad hoc" (sin cuenta de Apple Developer): la primera vez
# hay que abrirla con clic derecho > Abrir. Ver README.
set -euo pipefail

VERSION="${1:?falta la versión}"
OUT="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

APP="$WORK/Surco.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create \
  "$ROOT/target/x86_64-apple-darwin/release/surco" \
  "$ROOT/target/aarch64-apple-darwin/release/surco" \
  -output "$APP/Contents/MacOS/surco"
chmod 755 "$APP/Contents/MacOS/surco"
cp "$ROOT/packaging/icons/surco.icns" "$APP/Contents/Resources/surco.icns"
sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Firma ad hoc: Apple Silicon no corre binarios sin firmar.
codesign --force --deep --sign - "$APP"
codesign --verify --verbose "$APP"
lipo -info "$APP/Contents/MacOS/surco"

mkdir -p "$OUT"
# .zip con la app (para quien no quiera el .dmg).
ditto -c -k --keepParent "$APP" "$OUT/Surco-$VERSION-macos-universal.zip"

# .dmg con un acceso directo a /Applications para arrastrar.
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Surco" -srcfolder "$STAGE" -ov -format UDZO \
  "$OUT/Surco-$VERSION-macos-universal.dmg"
echo "listo: $OUT/Surco-$VERSION-macos-universal.dmg"
