#!/usr/bin/env python3
"""Generate the 1024x1024 app icon. Run from anywhere; output path is relative to this file."""
import math
import os

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "FarmHelper", "Resources", "Assets.xcassets", "AppIcon.appiconset", "AppIcon-1024.png")

top = (46, 125, 50)
bottom = (20, 64, 32)
image = Image.new("RGB", (SIZE, SIZE))
draw = ImageDraw.Draw(image)
for y in range(SIZE):
    t = y / (SIZE - 1)
    color = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
    draw.line([(0, y), (SIZE, y)], fill=color)

glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
gd = ImageDraw.Draw(glow)
gd.ellipse((150, 90, 874, 814), fill=(255, 255, 255, 30))
glow = glow.filter(ImageFilter.GaussianBlur(70))
image = Image.alpha_composite(image.convert("RGBA"), glow)

points = []
for i in range(101):
    t = i / 100
    y = 850 - 700 * t
    w = 250 * (math.sin(math.pi * t) ** 0.9)
    points.append((512 + w, y))
for i in range(100, -1, -1):
    t = i / 100
    y = 850 - 700 * t
    w = 250 * (math.sin(math.pi * t) ** 0.9)
    points.append((512 - w, y))

leaf = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
ld = ImageDraw.Draw(leaf)
ld.polygon(points, fill=(255, 255, 255, 255))
ld.line([(512, 850), (512, 915)], fill=(255, 255, 255, 255), width=24)
ld.line([(512, 820), (512, 190)], fill=(31, 94, 46, 255), width=18)
for yy in (660, 570, 480, 390):
    ld.line([(512, yy), (632, yy - 78)], fill=(31, 94, 46, 255), width=12)
    ld.line([(512, yy), (392, yy - 78)], fill=(31, 94, 46, 255), width=12)

image = Image.alpha_composite(image, leaf).convert("RGB")
os.makedirs(os.path.dirname(OUT), exist_ok=True)
image.save(OUT, "PNG")
print(OUT)
