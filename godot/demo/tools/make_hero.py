#!/usr/bin/env python3
"""Writes sprites/hero.png and sprites/hero.tres: a tiny SpriteFrames laid out like Pixmith's File > Export > Godot
(an atlas PNG, AtlasTexture sub-resources with a region, animations with frames {duration, texture}, speed, loop).
Standard library only: python3 godot/demo/tools/make_hero.py"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(HERE), "sprites")
RES_DIR = "res://sprites"
SIZE = 16
COLS = 3

SKIN, HAIR, SHIRT, PANTS, SHOE = (240, 190, 140), (90, 50, 30), (60, 110, 220), (40, 40, 70), (20, 20, 20)


def rect(img, x, y, w, h, c):
    for yy in range(y, y + h):
        for xx in range(x, x + w):
            img[yy][xx] = c + (255,)


def frame(bob, left, right):
    """One 16x16 frame. bob moves the body down a pixel, left/right lift a foot."""
    img = [[(0, 0, 0, 0)] * SIZE for _ in range(SIZE)]
    rect(img, 5, 1 + bob, 6, 2, HAIR)
    rect(img, 5, 3 + bob, 6, 4, SKIN)
    rect(img, 6, 4 + bob, 1, 1, (0, 0, 0))
    rect(img, 9, 4 + bob, 1, 1, (0, 0, 0))
    rect(img, 4, 7 + bob, 8, 4, SHIRT)
    rect(img, 3, 8 + bob, 1, 3, SKIN)
    rect(img, 12, 8 + bob, 1, 3, SKIN)
    rect(img, 5, 11 + bob, 2, 3 - left, PANTS)
    rect(img, 9, 11 + bob, 2, 3 - right, PANTS)
    rect(img, 4, 14 - left, 3, 1, SHOE)
    rect(img, 9, 14 - right, 3, 1, SHOE)
    return img


# (name, loop, frame time in ms, [(bob, left foot lift, right foot lift)])
ANIMS = [
    ("idle", True, 400, [(0, 0, 0), (1, 0, 0)]),
    ("walk", True, 120, [(0, 0, 0), (1, 2, 0), (0, 0, 0), (1, 0, 2)]),
]


def png(w, h, rows):
    raw = b"".join(b"\x00" + bytes(v for px in row for v in px) for row in rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def main():
    frames = [(a, i, f) for a in ANIMS for i, f in enumerate(a[3])]
    w, h = COLS * SIZE, ((len(frames) + COLS - 1) // COLS) * SIZE
    atlas = [[(0, 0, 0, 0)] * w for _ in range(h)]
    pos = {}
    for n, (a, i, f) in enumerate(frames):
        x, y = (n % COLS) * SIZE, (n // COLS) * SIZE
        pos[(a[0], i)] = (x, y)
        img = frame(*f)
        for yy in range(SIZE):
            atlas[y + yy][x:x + SIZE] = img[yy]
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, "hero.png"), "wb") as fh:
        fh.write(png(w, h, atlas))

    # Same text layout as core/src/gamedev/godot/sprite_frames.rs `build`.
    t = ['[gd_resource type="SpriteFrames" format=3]\n\n',
         '[ext_resource type="Texture2D" path="%s/hero.png" id="1_tex"]\n\n' % RES_DIR]
    for name, _, _, fr in ANIMS:
        for i in range(len(fr)):
            x, y = pos[(name, i)]
            t.append('[sub_resource type="AtlasTexture" id="at_%s_%d"]\natlas = ExtResource("1_tex")\n'
                     'region = Rect2(%d, %d, %d, %d)\n\n' % (name, i, x, y, SIZE, SIZE))
    t.append("[resource]\nanimations = [")
    for ai, (name, loop, ms, fr) in enumerate(ANIMS):
        t.append("{\n" if ai == 0 else ", {\n")
        t.append('"frames": [')
        for i in range(len(fr)):
            t.append("{\n" if i == 0 else ", {\n")
            t.append('"duration": 1.0,\n"texture": SubResource("at_%s_%d")\n}' % (name, i))
        t.append("],\n")
        t.append('"loop": %s,\n"name": &"%s",\n"speed": %r\n}' % ("true" if loop else "false", name, 1000.0 / ms))
    t.append("]\n")
    with open(os.path.join(OUT, "hero.tres"), "w") as fh:
        fh.write("".join(t))
    print("wrote", OUT)


if __name__ == "__main__":
    main()
