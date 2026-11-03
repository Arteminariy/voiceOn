#!/usr/bin/env python3
"""Генератор иконки приложения: зелёная волна на тёмной плитке.
Запуск: python3 gen_icon.py  (нужен Pillow) -> icon.png (256x256) и icon.ico."""
from PIL import Image, ImageDraw

S = 256 * 4  # суперсэмплинг для сглаживания
BG, GREEN = (21, 23, 27, 255), (61, 220, 132, 255)

img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.22), fill=BG)

# Пути из макета: viewBox 24x24, штрихи x=4,8,12,16,20, высоты как в макете.
bars = [(4, 10, 14), (8, 7, 17), (12, 4, 20), (16, 7, 17), (20, 10, 14)]
scale = S * 0.62 / 24          # рисунок занимает ~62% плитки
off = (S - 24 * scale) / 2
w = 2.6 * scale                # толщина штриха
for x, y0, y1 in bars:
    cx = off + x * scale
    d.rounded_rectangle([cx - w / 2, off + y0 * scale - w / 2, cx + w / 2, off + y1 * scale + w / 2],
                        radius=w / 2, fill=GREEN)

img = img.resize((256, 256), Image.LANCZOS)
img.save("icon.png")
img.save("icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
