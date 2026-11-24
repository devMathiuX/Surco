#!/bin/sh
# Instala Surco desde su repositorio APT (Ubuntu, Debian, Linux Mint, Pop!_OS…).
#
#   curl -fsSL @BASE_URL@/install.sh | sudo sh
#
# Agrega la clave y el repositorio una sola vez. Después Surco se actualiza
# con el resto del sistema (sudo apt upgrade o el Actualizador de software).
set -eu

REPO="@BASE_URL@"
KEYRING=/etc/apt/keyrings/surco.gpg
SOURCES=/etc/apt/sources.list.d/surco.sources

if [ "$(id -u)" -ne 0 ]; then
    echo "Corré este instalador con sudo:  curl -fsSL $REPO/install.sh | sudo sh" >&2
    exit 1
fi
if ! command -v apt-get >/dev/null 2>&1; then
    echo "Este instalador es para Ubuntu, Debian y derivadas (usan apt)." >&2
    echo "En otras distros usá el AppImage: https://github.com/devMathiuX/Surco/releases" >&2
    exit 1
fi
arch=$(dpkg --print-architecture)
if [ "$arch" != amd64 ]; then
    echo "Surco por ahora solo tiene paquete para amd64 (esta máquina es $arch)." >&2
    exit 1
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
else
    echo "Hace falta curl o wget." >&2
    exit 1
fi

echo "==> Agregando la clave del repositorio de Surco"
install -d -m 0755 /etc/apt/keyrings
tmp=$(mktemp)
fetch "$REPO/surco.gpg" "$tmp"
install -m 0644 "$tmp" "$KEYRING"
rm -f "$tmp"

echo "==> Agregando el repositorio ($SOURCES)"
cat > "$SOURCES" <<SRC
Types: deb
URIs: $REPO
Suites: stable
Components: main
Architectures: amd64
Signed-By: $KEYRING
SRC

echo "==> Instalando Surco"
# Si otro repositorio de la máquina falla, seguimos igual: el de Surco ya se bajó.
apt-get update || echo "Aviso: apt-get update tuvo errores con otro repositorio; sigo igual." >&2
apt-get install -y surco

echo
echo "Listo. Abrí Surco desde el menú de aplicaciones o escribiendo: surco"
echo "Se actualiza solo con: sudo apt update && sudo apt upgrade"
