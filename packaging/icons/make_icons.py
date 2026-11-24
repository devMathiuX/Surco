#!/usr/bin/env python3
"""Genera el ícono de Surco en todos los tamaños (necesita Pillow).

Uso: python3 packaging/icons/make_icons.py packaging/icons
"""
import sys
from PIL import Image, ImageDraw
S=4096  # supersampling (final 1024)
img=Image.new("RGBA",(S,S),(0,0,0,0))
d=ImageDraw.Draw(img)
pad=int(S*0.06)
d.rounded_rectangle([pad,pad,S-pad,S-pad],radius=int(S*0.2),fill=(0x1A,0x1A,0x1A,255))
cx=cy=S//2
# surcos: anillos concéntricos
for i,r in enumerate(range(int(S*0.12), int(S*0.37), int(S*0.045))):
    w=int(S*0.012)
    d.ellipse([cx-r,cy-r,cx+r,cy+r],outline=(0x3A,0x3A,0x3A,255),width=w)
# un surco destacado en acento (arco de 3/4)
r=int(S*0.345); w=int(S*0.03)
d.arc([cx-r,cy-r,cx+r,cy+r],start=-200,end=40,fill=(0x7C,0x9C,0xFF,255),width=w)
r2=int(S*0.255)
d.arc([cx-r2,cy-r2,cx+r2,cy+r2],start=-160,end=-20,fill=(0x7C,0x9C,0xFF,170),width=int(S*0.022))
# centro
r=int(S*0.06)
d.ellipse([cx-r,cy-r,cx+r,cy+r],fill=(0x7C,0x9C,0xFF,255))
r=int(S*0.018)
d.ellipse([cx-r,cy-r,cx+r,cy+r],fill=(0x11,0x11,0x11,255))
out=sys.argv[1]
big=img.resize((1024,1024),Image.LANCZOS)
big.save(f"{out}/surco-1024.png")
for s in (512,256,128,64,48,32,16):
    big.resize((s,s),Image.LANCZOS).save(f"{out}/surco-{s}.png")
big.save(f"{out}/surco.ico",sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
big.save(f"{out}/surco.icns")
