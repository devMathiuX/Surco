#!/usr/bin/env python3
"""Genera el ícono de Surco en todos los tamaños (necesita Pillow).

Un vinilo visto de frente: disco negro con surcos y brillo, etiqueta del color
de acento y una "S" hecha de dos semicírculos (el surco que da nombre a la
app). La app dibuja lo mismo en vectorial con el acento del tema
(`crates/app/src/icons.rs`, `logo`).

Uso: python3 packaging/icons/make_icons.py packaging/icons
"""
import sys
from PIL import Image, ImageDraw

ACCENT = (0x7C, 0x9C, 0xFF, 255)
DISC = (0x14, 0x14, 0x16, 255)
GROOVE = (0x2C, 0x2C, 0x30, 255)
INK = (0x14, 0x14, 0x16, 255)


def draw(size, accent=ACCENT):
    S = size * 4  # supersampling
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = S / 2
    R = S * 0.48

    def circle(r, **kw):
        d.ellipse([c - r, c - r, c + r, c + r], **kw)

    def arc(cx, cy, r, a, b, w, fill):
        # `r` es el radio del centro del trazo (PIL dibuja el ancho hacia adentro).
        o = r + w / 2
        d.arc([cx - o, cy - o, cx + o, cy + o], start=a, end=b, fill=fill, width=int(w))

    circle(R, fill=DISC)
    # Surcos.
    for k in (0.86, 0.74, 0.62):
        circle(R * k, outline=GROOVE, width=max(1, int(S * 0.012)))
    # Brillo del vinilo: dos cuñas claras y suaves, opuestas.
    sheen = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    r = R * 0.95
    for a in (200, 20):
        sd.pieslice([c - r, c - r, c + r, c + r], start=a, end=a + 50, fill=(255, 255, 255, 26))
    img.alpha_composite(sheen)
    # Etiqueta.
    L = R * 0.47
    circle(L, fill=accent)
    # "S": dos semicírculos que se tocan en el centro.
    r = L * 0.36
    w = L * 0.20
    arc(c, c - r, r, 90, 330, w, INK)
    arc(c, c + r, r, 270, 510, w, INK)
    return img.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    out = sys.argv[1]
    big = draw(1024)
    big.save(f"{out}/surco-1024.png")
    for s in (512, 256, 128, 64, 48, 32, 16):
        draw(s).save(f"{out}/surco-{s}.png")
    big.save(f"{out}/surco.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    big.save(f"{out}/surco.icns")
