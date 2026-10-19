# Copyright © SixtyFPS GmbH <info@slint.dev>
# SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

"""Generate a COLRv0 regression font with translucent red and foreground layers.

Requires fontTools; the generated font is checked in, so tests need no Python.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.colorLib.builder import buildCOLR, buildCPAL

builder = FontBuilder(1000, isTTF=True)
builder.setupGlyphOrder(['.notdef', 'emoji', 'red', 'foreground'])
builder.setupCharacterMap({0x1F600: 'emoji'})
glyphs = {}
for name, left, right in [('.notdef', 0, 0), ('emoji', 0, 0), ('red', 0, 500), ('foreground', 500, 1000)]:
    pen = TTGlyphPen(None)
    if right:
        pen.moveTo((left, 0))
        pen.lineTo((right, 0))
        pen.lineTo((right, 1000))
        pen.lineTo((left, 1000))
        pen.closePath()
    glyphs[name] = pen.glyph()
builder.setupGlyf(glyphs)
builder.setupHorizontalMetrics({name: (1000, 500 if name == 'foreground' else 0) for name in glyphs})
builder.setupHorizontalHeader(ascent=1000, descent=0)
builder.setupNameTable({'familyName': 'COLR Test', 'styleName': 'Regular'})
builder.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
builder.setupPost()
builder.setupMaxp()
builder.font['COLR'] = buildCOLR({'emoji': [('red', 0), ('foreground', 0xFFFF)]}, version=0)
builder.font['CPAL'] = buildCPAL([[(1, 0, 0, 0.5)]])
builder.save(Path(__file__).with_suffix('.ttf'))
