#!/usr/bin/env bash
# Arma la página de Surco (la landing) dentro del sitio de GitHub Pages.
#
#   web/build.sh <carpeta-del-sitio> <url-base>
#
# - <carpeta-del-sitio> es lo que se publica en la rama gh-pages. Puede traer el
#   repo APT (dists/, pool/, install.sh, surco.gpg...): no se toca, solo se
#   agregan o pisan index.html, favicon.png, img/, release.json y apt/index.html.
# - <url-base> es donde queda publicado, p. ej. https://devmathiux.github.io/Surco
# - Si hay GITHUB_REPOSITORY y gh, escribe release.json con el último Release
#   (la página lo lee para los links de descarga; si falta, usa la API de GitHub).
set -euo pipefail

site=$1
base_url=${2%/}
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")

mkdir -p "$site"
cp -f "$here/index.html" "$here/favicon.png" "$site/"
rm -rf "$site/img"
cp -r "$here/img" "$site/img"
touch "$site/.nojekyll"

if [ -n "${GITHUB_REPOSITORY:-}" ] && command -v gh >/dev/null; then
    if gh api "repos/$GITHUB_REPOSITORY/releases/latest" \
        --jq '{tag_name, html_url, assets: [.assets[] | {name, size, browser_download_url}]}' \
        > "$site/release.json.tmp"; then
        mv "$site/release.json.tmp" "$site/release.json"
        echo "release.json: $(grep -o '"tag_name":"[^"]*"' "$site/release.json")"
    else
        rm -f "$site/release.json.tmp"
        echo "::warning::No pude leer el último Release: la página va a usar la API de GitHub."
    fi
fi

# Página del repo APT en apt/ (la raíz es la landing). Solo si el repo existe.
shopt -s nullglob
debs=("$site"/pool/main/s/surco/*.deb)
if [ ${#debs[@]} -gt 0 ]; then
    latest=$(for f in "${debs[@]}"; do dpkg-deb -f "$f" Version; done | sort -V | tail -n1)
    mkdir -p "$site/apt"
    sed -e "s|@BASE_URL@|$base_url|g" -e "s|@VERSION@|$latest|g" \
        "$root/packaging/linux/apt/index.html" > "$site/apt/index.html"
fi

echo "Página lista en $site"
