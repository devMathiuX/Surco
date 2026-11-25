#!/usr/bin/env python3
"""Genera el ícono de Surco en todos los tamaños (necesita Pillow).

Un vinilo con surcos en degradado, la etiqueta y el eje con borde del color
de acento, y el brazo del tocadiscos apoyado arriba a la derecha. Es el mismo
dibujo (de 256×256) que la app hace en vectorial con el acento del tema
(`crates/app/src/icons.rs`, `logo`).

Uso: python3 packaging/icons/make_icons.py packaging/icons
"""
import math
import sys

from PIL import Image, ImageDraw

DISC = (0x10, 0x13, 0x1A, 255)
RIM = (0x28, 0x31, 0x42, 255)
LABEL = (0x14, 0x18, 0x20, 255)
ACCENT_FROM = (0x38, 0xBD, 0xF8)
ACCENT_TO = (0x25, 0x63, 0xEB)
GROOVE = [((0x60, 0xA5, 0xFA), 0.85), ((0x3B, 0x82, 0xF6), 0.30), ((0x38, 0xBD, 0xF8), 0.85)]


def gradient(S, k, a, b, stops):
    """Imagen S×S con un degradado lineal de `a` a `b` (coordenadas de 256)."""
    ax, ay = a[0] * k, a[1] * k
    dx, dy = (b[0] - a[0]) * k, (b[1] - a[1]) * k
    n = dx * dx + dy * dy
    small = 128
    img = Image.new("RGBA", (small, small))
    px = img.load()
    for y in range(small):
        for x in range(small):
            X, Y = x * S / small, y * S / small
            t = max(0.0, min(1.0, ((X - ax) * dx + (Y - ay) * dy) / n))
            # Interpolar entre las paradas.
            for i in range(len(stops) - 1):
                t0, c0, a0 = stops[i]
                t1, c1, a1 = stops[i + 1]
                if t <= t1:
                    u = (t - t0) / (t1 - t0) if t1 > t0 else 0
                    c = tuple(int(c0[j] + (c1[j] - c0[j]) * u) for j in range(3))
                    al = a0 + (a1 - a0) * u
                    break
            px[x, y] = c + (int(255 * al),)
    return img.resize((S, S), Image.BILINEAR)


def draw(size):
    S = size * 4  # supersampling
    k = S / 256
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = 128 * k

    def circle(draw, r, **kw):
        draw.ellipse([c - r * k, c - r * k, c + r * k, c + r * k], **kw)

    def stroked(paint, shape):
        """Pinta `paint` (imagen) a través de la máscara que dibuja `shape`."""
        mask = Image.new("L", (S, S), 0)
        shape(ImageDraw.Draw(mask))
        layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        layer.paste(paint, (0, 0), mask)
        img.alpha_composite(layer)

    accent = gradient(S, k, (76, 76), (180, 180), [(0, ACCENT_FROM, 1), (1, ACCENT_TO, 1)])
    arm = gradient(S, k, (196, 42), (172, 95), [(0, ACCENT_FROM, 1), (1, ACCENT_TO, 1)])
    groove = gradient(
        S, k, (40, 40), (216, 216),
        [(0, GROOVE[0][0], GROOVE[0][1]), (0.5, GROOVE[1][0], GROOVE[1][1]), (1, GROOVE[2][0], GROOVE[2][1])],
    )

    def ring(r, w):
        # El trazo va centrado en el radio, como en SVG.
        o = r + w / 2
        return lambda m: m.ellipse([c - o * k, c - o * k, c + o * k, c + o * k], outline=255, width=max(1, round(w * k)))

    circle(d, 114.5, fill=RIM)
    circle(d, 109.5, fill=DISC)
    for r, w in ((96, 2), (82, 2), (68, 2), (54, 1.75)):
        stroked(groove, ring(r, w))
    d = ImageDraw.Draw(img)
    circle(d, 36, fill=LABEL)
    stroked(accent, ring(36, 3.5))
    d = ImageDraw.Draw(img)
    circle(d, 8, fill=DISC)
    stroked(accent, ring(8, 3))

    # Brazo: línea con una curva corta al final.
    pts = [(196, 42), (187, 77)]
    for i in range(1, 9):
        t = i / 8
        # Curva cuadrática de (187,77) a (182.1,85.4) con control (185.6,82.6).
        x = (1 - t) ** 2 * 187 + 2 * (1 - t) * t * 185.6 + t * t * 182.1
        y = (1 - t) ** 2 * 77 + 2 * (1 - t) * t * 82.6 + t * t * 85.4
        pts.append((x, y))
    pts.append((172, 95))
    w = 6 * k

    def arm_shape(m):
        P = [(x * k, y * k) for x, y in pts]
        m.line(P, fill=255, width=round(w), joint="curve")
        for x, y in (P[0], P[-1]):
            m.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=255)

    stroked(arm, arm_shape)

    # Cápsula: rectángulo de 19×10 girado -43° alrededor de (155, 96).
    ang = math.radians(-43)

    def rot(x, y):
        return (
            (155 + x * math.cos(ang) - y * math.sin(ang)) * k,
            (96 + x * math.sin(ang) + y * math.cos(ang)) * k,
        )

    outer = [rot(-1.5, -1.5), rot(20.5, -1.5), rot(20.5, 11.5), rot(-1.5, 11.5)]
    inner = [rot(1.5, 1.5), rot(17.5, 1.5), rot(17.5, 8.5), rot(1.5, 8.5)]
    d = ImageDraw.Draw(img)
    d.polygon(outer, fill=ACCENT_FROM + (255,))
    d.polygon(inner, fill=LABEL)
    d.line([(158 * k, 106 * k), (153 * k, 111 * k)], fill=(0x60, 0xA5, 0xFA, 255), width=round(3 * k))
    return img.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    out = sys.argv[1]
    big = draw(1024)
    big.save(f"{out}/surco-1024.png")
    for s in (512, 256, 128, 64, 48, 32, 16):
        draw(s).save(f"{out}/surco-{s}.png")
    big.save(f"{out}/surco.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    big.save(f"{out}/surco.icns")
