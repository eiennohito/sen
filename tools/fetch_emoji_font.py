#!/usr/bin/env python3
"""Fetch the current bitmap Noto Color Emoji font and OFL licence from jsDelivr.

Structural validation requires fontTools (python3-fonttools).
"""
from io import BytesIO
import os
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
FONT_DIR = ROOT / 'fonts'
SOURCE = 'https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/2D/fonts/'
FILES = (('NotoColorEmoji.ttf', 'NotoColorEmoji.ttf'), ('LICENSE', 'OFL.txt'))
CAP = 50 * 1024 * 1024


def validate(data):
    try:
        from fontTools.ttLib import TTFont, TTLibError
    except ImportError as error:
        raise ValueError('Install python3-fonttools to validate the emoji font') from error
    try:
        with TTFont(BytesIO(data)) as font:
            if not {'CBDT', 'CBLC'}.issubset(font.keys()):
                raise ValueError('Missing CBDT/CBLC tables')
            # Decode bitmap tables too, so malformed tables are rejected.
            font['CBDT']
            font['CBLC']
            cmap = font.getBestCmap() or {}
            if not all(cmap.get(ord(ch)) not in (None, '.notdef') for ch in '😀🌸❤'):
                raise ValueError('Missing emoji cmap entries')
    except (TTLibError, KeyError, IndexError) as error:
        raise ValueError('Invalid emoji font') from error


def verified_files():
    for _, name in FILES:
        path = FONT_DIR / name
        data = path.read_bytes()
        if name.endswith('.ttf'):
            validate(data)
        elif b'SIL OPEN FONT LICENSE' not in data:
            raise ValueError('Invalid OFL licence')
        yield path


class CDNOnly(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('Unexpected CDN redirect')


def main():
    FONT_DIR.mkdir(exist_ok=True)
    opener = urllib.request.build_opener(CDNOnly)
    for source, name in FILES:
        with opener.open(SOURCE + source, timeout=60) as response:
            data = response.read(CAP + 1)
        if len(data) > CAP:
            raise ValueError(f'File too large: {name}')
        if name.endswith('.ttf'):
            validate(data)
        elif b'SIL OPEN FONT LICENSE' not in data:
            raise ValueError('Invalid OFL licence')
        path = FONT_DIR / name
        temporary = path.with_suffix('.tmp')
        try:
            with temporary.open('wb') as output:
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
