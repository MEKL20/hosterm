#!/usr/bin/env python3
"""Generate the hosterm app icon (1024x1024 PNG). Tokyo Night palette to match the UI.
Mark: rounded dark square + terminal prompt chevron '>' + cursor block = terminal/host.
"""
from PIL import Image, ImageDraw

S = 1024
BG = (26, 27, 38)       # #1a1b26  app background
BG2 = (31, 35, 53)      # #1f2335  slightly lighter inner
ACCENT = (122, 162, 247)  # #7aa2f7 blue
FG = (192, 202, 245)    # #c0caf5

img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# rounded-square background
pad = 72
radius = 180
d.rounded_rectangle([pad, pad, S - pad, S - pad], radius=radius, fill=BG)
# subtle inner border for depth
d.rounded_rectangle([pad + 10, pad + 10, S - pad - 10, S - pad - 10],
                    radius=radius - 10, outline=BG2, width=14)

# terminal prompt chevron ">"  (two strokes)
cx, cy = 478, 512          # elbow of the chevron (optically centered: mark bbox 328..698)
arm = 150                  # arm length
w = 54                     # stroke thickness
# upper arm: from top-left down to elbow
d.line([(cx - arm, cy - arm), (cx, cy)], fill=ACCENT, width=w, joint="curve")
# lower arm: from elbow up to bottom-left mirror
d.line([(cx, cy), (cx - arm, cy + arm)], fill=ACCENT, width=w, joint="curve")
# round the stroke ends
for px, py in [(cx - arm, cy - arm), (cx, cy), (cx - arm, cy + arm)]:
    d.ellipse([px - w // 2, py - w // 2, px + w // 2, py + w // 2], fill=ACCENT)

# cursor block to the right of the chevron
bx0, by0 = cx + 70, cy - 70
bw, bh = 150, 150
d.rounded_rectangle([bx0, by0, bx0 + bw, by0 + bh], radius=24, fill=FG)

img.save("scripts/icon-src.png")
print("wrote scripts/icon-src.png", img.size)
