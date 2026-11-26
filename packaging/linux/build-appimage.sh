#!/usr/bin/env bash
# Arma Surco-<versión>-x86_64.AppImage a partir de target/release/surco.
# Uso: packaging/linux/build-appimage.sh <versión> [carpeta de salida]
#
# Necesita `appimagetool` en el PATH (el workflow de release lo baja).
# No se empaquetan bibliotecas: Surco solo necesita libasound2 y libxkbcommon,
# que vienen con cualquier escritorio. Compilar en la distro más vieja que se
# quiera soportar (el CI usa Ubuntu 22.04, glibc 2.35).
set -euo pipefail

VERSION="${1:?falta la versión}"
OUT="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

APPDIR="$WORK/Surco.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$ROOT/target/release/surco" "$APPDIR/usr/bin/surco"
install -m 644 "$ROOT/packaging/linux/surco.desktop" "$APPDIR/usr/share/applications/surco.desktop"
install -m 644 "$ROOT/packaging/icons/surco-256.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/surco.png"
# Licencia de Surco y de sus dependencias (THIRD_PARTY_LICENSES.html lo arma
# `cargo about`, ver about.toml).
mkdir -p "$APPDIR/usr/share/doc/surco"
install -m 644 "$ROOT/LICENSE" "$ROOT/THIRD_PARTY_LICENSES.html" "$APPDIR/usr/share/doc/surco/"
# appimagetool quiere el .desktop y el ícono en la raíz, y un AppRun.
cp "$APPDIR/usr/share/applications/surco.desktop" "$APPDIR/surco.desktop"
cp "$ROOT/packaging/icons/surco-256.png" "$APPDIR/surco.png"
ln -s usr/bin/surco "$APPDIR/AppRun"

mkdir -p "$OUT"
# Sin FUSE (contenedores, CI): que appimagetool se extraiga y corra.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 appimagetool --no-appstream "$APPDIR" \
  "$OUT/Surco-$VERSION-x86_64.AppImage"
echo "listo: $OUT/Surco-$VERSION-x86_64.AppImage"
