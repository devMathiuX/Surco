"""Escribe en index.html la versión y los links del último Release.

    python3 web/stamp.py <index.html> <release.json>

Usa los mismos patrones que MATCH en el <script> de la página.
"""
import json
import re
import sys

MATCH = {
    "setup": r"windows-x64-setup\.exe$",
    "winzip": r"windows-x64-portable\.zip$",
    "dmg": r"macos-universal\.dmg$",
    "maczip": r"macos-universal\.zip$",
    "deb": r"_amd64\.deb$",
    "appimage": r"\.AppImage$",
    "targz": r"linux-x86_64\.tar\.gz$",
}

page_path, rel_path = sys.argv[1], sys.argv[2]
page = open(page_path, encoding="utf-8").read()
rel = json.load(open(rel_path, encoding="utf-8"))

version = str(rel.get("tag_name", "")).removeprefix("v")
if version:
    page = re.sub(r"(<b data-ver>)[^<]*(</b>)", rf"\g<1>{version}\g<2>", page)
if rel.get("html_url"):
    page = re.sub(r'(<a class="badge" id="badge" href=")[^"]*', rf"\g<1>{rel['html_url']}", page)


def size(b):
    return f"{b / 1048576:.1f}".replace(".", ",") + " MB"


for key, pattern in MATCH.items():
    asset = next((a for a in rel.get("assets", []) if re.search(pattern, a["name"], re.I)), None)
    if not asset:
        continue

    def fill(m, asset=asset):
        block = m.group(0)
        block = re.sub(r'href="[^"]*"', f'href="{asset["browser_download_url"]}"', block, count=1)
        block = re.sub(r"(<span data-name>)[^<]*", rf"\g<1>{asset['name']}", block)
        block = re.sub(r"(<span class=\"sz\" data-size>)[^<]*", rf"\g<1>{size(asset['size'])}", block)
        return block

    page = re.sub(rf'<a class="file[^"]*" data-asset="{key}".*?</a>', fill, page, flags=re.S)

open(page_path, "w", encoding="utf-8").write(page)
print(f"index.html: versión {version or '?'}")
