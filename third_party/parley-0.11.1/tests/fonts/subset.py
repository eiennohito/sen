# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Generate fixtures with fontTools from verified local inputs (no downloads).

Usage: subset.py /path/to/DejaVuSans.ttf
Text source: DejaVu Sans 2.37, dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf
from https://github.com/dejavu-fonts/dejavu-fonts/releases/tag/version_2_37
Emoji source: Noto Color Emoji pinned by tools/fetch_emoji_font.py.
"""
from pathlib import Path
import hashlib
import sys

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.ttLib.scaleUpem import scale_upem

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / 'tools'))
from fetch_emoji_font import verified_files

TEXT_SHA256 = '6038a160b491e121c1f12c7bccb4a9c8730296e3adc1086a059404ed84b7451c'
SAMPLE = '🌸😀☕❤️🐈👍👩\u200d💻🇯🇵1️⃣☺\ufe0e#©ab'


def make_subset(source, destination, family=None):
    font = TTFont(source, recalcTimestamp=False)
    scale_upem(font, 2048)
    options = subset.Options()
    options.recalc_timestamp = False
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=SAMPLE)
    subsetter.subset(font)
    if family:
        for record in font['name'].names:
            if record.nameID in (1, 4, 6, 16):
                name = family.replace(' ', '') if record.nameID == 6 else family
                record.string = name.encode(record.getEncoding())
    font.save(destination)


text = Path(sys.argv[1])
if hashlib.sha256(text.read_bytes()).hexdigest() != TEXT_SHA256:
    raise ValueError('Expected DejaVu Sans 2.37 with SHA256 ' + TEXT_SHA256)
emoji, _ = verified_files()
make_subset(emoji, HERE / 'emoji.ttf')
make_subset(text, HERE / 'text.ttf', 'Test Text')
