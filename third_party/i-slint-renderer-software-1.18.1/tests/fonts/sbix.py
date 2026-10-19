# Copyright © SixtyFPS GmbH <info@slint.dev>
# SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

"""Generate an sbix font with straight-alpha PNG edge pixels (fontTools and Pillow)."""
from io import BytesIO
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables.sbixGlyph import Glyph
from fontTools.ttLib.tables.sbixStrike import Strike
from PIL import Image

builder = FontBuilder(1000, isTTF=True)
builder.setupGlyphOrder(['.notdef', 'emoji'])
builder.setupCharacterMap({0x1F600: 'emoji'})
builder.setupGlyf({name: TTGlyphPen(None).glyph() for name in ['.notdef', 'emoji']})
builder.setupHorizontalMetrics({name: (1000, 0) for name in ['.notdef', 'emoji']})
builder.setupHorizontalHeader(ascent=1000, descent=0)
builder.setupNameTable({'familyName': 'sbix Test', 'styleName': 'Regular'})
builder.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
builder.setupPost()
builder.setupMaxp()
image = Image.new('RGBA', (20, 20), (255, 80, 20, 255))
for x in range(20):
    for y in range(20):
        if x in (0, 19) or y in (0, 19):
            image.putpixel((x, y), (255, 80, 20, 64))
png = BytesIO()
image.save(png, format='PNG')
strike = Strike(ppem=20, resolution=72)
strike.glyphs['emoji'] = Glyph(glyphName='emoji', graphicType='png ', imageData=png.getvalue())
builder.font['sbix'] = newTable('sbix')
builder.font['sbix'].strikes[20] = strike
builder.save(Path(__file__).with_suffix('.ttf'))
