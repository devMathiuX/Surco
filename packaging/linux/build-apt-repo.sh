#!/usr/bin/env bash
# Arma (o actualiza) el repositorio APT de Surco que se publica en GitHub Pages.
#
#   packaging/linux/build-apt-repo.sh <carpeta-con-.deb-nuevos> <carpeta-del-sitio> <url-base>
#
# - <carpeta-del-sitio> puede traer el repo anterior (rama gh-pages): los .deb
#   viejos se conservan y se dejan solo las últimas $SURCO_APT_KEEP versiones.
# - Firma con la clave GPG por defecto del keyring, o con $SURCO_APT_KEY
#   (id o huella). Si la clave tiene contraseña, va en $SURCO_APT_PASSPHRASE.
# - <url-base> es donde queda publicado, p. ej. https://devmathiux.github.io/Surco
#   (se usa en install.sh y en la página).
#
# Estructura que queda (suite "stable", componente "main", solo amd64):
#   dists/stable/{Release,InRelease,Release.gpg}
#   dists/stable/main/binary-amd64/Packages{,.gz}
#   pool/main/s/surco/surco_<v>-1_amd64.deb
#   surco.gpg, surco.asc, install.sh, apt/index.html
# (la raíz del sitio es la página de Surco, que arma web/build.sh)
set -euo pipefail

new_debs=$1
site=$2
base_url=${3%/}
keep=${SURCO_APT_KEEP:-3}
here=$(cd "$(dirname "$0")" && pwd)

command -v apt-ftparchive >/dev/null || { echo "Falta apt-ftparchive (paquete apt-utils)" >&2; exit 1; }

shopt -s nullglob
debs=("$new_debs"/*.deb)
[ ${#debs[@]} -gt 0 ] || { echo "No hay .deb en $new_debs" >&2; exit 1; }

pool="$site/pool/main/s/surco"
mkdir -p "$pool"
for deb in "${debs[@]}"; do
    pkg=$(dpkg-deb -f "$deb" Package)
    [ "$pkg" = surco ] || { echo "$deb no es el paquete surco ($pkg)" >&2; exit 1; }
    cp -f "$deb" "$pool/"
done

# Dejar solo las últimas $keep versiones (las viejas siguen en GitHub Releases).
mapfile -t all < <(for f in "$pool"/*.deb; do echo "$(dpkg-deb -f "$f" Version) $f"; done \
    | sort -k1,1 -V -r | cut -d' ' -f2-)
for f in "${all[@]:$keep}"; do
    echo "Saco del repo: $(basename "$f")"
    rm -f "$f"
done

# Índices.
dist="$site/dists/stable"
rm -rf "$dist"
mkdir -p "$dist/main/binary-amd64"
(cd "$site" && apt-ftparchive packages pool) > "$dist/main/binary-amd64/Packages"
gzip -9nk "$dist/main/binary-amd64/Packages"
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=Surco \
    -o APT::FTPArchive::Release::Label=Surco \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Codename=stable \
    -o APT::FTPArchive::Release::Architectures=amd64 \
    -o APT::FTPArchive::Release::Components=main \
    -o APT::FTPArchive::Release::Description="Surco: playlists de Spotify con audio de YouTube" \
    release "$dist" > "$site/Release.tmp"
mv "$site/Release.tmp" "$dist/Release"

# Firma: InRelease (apt moderno) y Release.gpg (apt viejo).
gpg_args=(--batch --yes --digest-algo SHA512)
[ -n "${SURCO_APT_KEY:-}" ] && gpg_args+=(--local-user "$SURCO_APT_KEY")
if [ -n "${SURCO_APT_PASSPHRASE:-}" ]; then
    gpg_args+=(--pinentry-mode loopback --passphrase "$SURCO_APT_PASSPHRASE")
fi
gpg "${gpg_args[@]}" --clearsign -o "$dist/InRelease" "$dist/Release"
gpg "${gpg_args[@]}" --armor --detach-sign -o "$dist/Release.gpg" "$dist/Release"

# Clave pública (binaria para /etc/apt/keyrings y armada para verla en el navegador).
key=${SURCO_APT_KEY:-$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')}
gpg --batch --yes --export -o "$site/surco.gpg" "$key"
gpg --batch --yes --armor --export -o "$site/surco.asc" "$key"

# Instalador y página.
latest=$(dpkg-deb -f "${all[0]}" Version)
sed "s|@BASE_URL@|$base_url|g" "$here/apt/install.sh" > "$site/install.sh"
mkdir -p "$site/apt"
sed -e "s|@BASE_URL@|$base_url|g" -e "s|@VERSION@|$latest|g" "$here/apt/index.html" > "$site/apt/index.html"
touch "$site/.nojekyll"

echo "Repo APT listo en $site (última versión: $latest)"
ls -la "$pool"
