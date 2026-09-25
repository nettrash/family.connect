#!/usr/bin/env python3
"""Is this window capture a picture, or an empty frame?

`screencapture -l <window id>` copies a window's backing store. A window
that has never been DISPLAYED — off the active Space, or still being drawn —
has none, and the result is a transparent or single-colour frame that the
compositing step turns into a blank blue rectangle, indistinguishable at a
glance from a real screenshot in a file listing. One Mac screenshot came out
that way on 2026-09-17 while five others came out right.

The test is deliberately crude, because the failure is not subtle: downscale
and count distinct colours. A drawn window has hundreds; an empty one has a
handful. Exit 0 if it is worth keeping.
"""
import sys

from PIL import Image

MIN_COLOURS = 24

path = sys.argv[1]
im = Image.open(path).convert("RGBA")
# A fully transparent frame is the commonest empty case, and counting
# colours would see exactly one.
alpha = im.getchannel("A")
if alpha.getextrema()[1] == 0:
    print(f"  {path}: every pixel transparent — the window was never drawn")
    sys.exit(1)
small = im.convert("RGB").resize((120, 120), Image.NEAREST)
colours = len(set(small.getdata()))
if colours < MIN_COLOURS:
    print(f"  {path}: {colours} distinct colours — an empty frame, not a screen")
    sys.exit(1)
sys.exit(0)
