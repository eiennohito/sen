#!/usr/bin/env python3
"""Fetch the pinned Noto Color Emoji 2.057 font and OFL-1.1 licence."""
import hashlib
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
FONT_DIR = ROOT / 'fonts'
SOURCE = 'https://raw.githubusercontent.com/googlefonts/noto-emoji/v2026-09-24-unicode18_0/2D/fonts/'
FILES = (
    ('NotoColorEmoji.ttf', 'NotoColorEmoji.ttf', '15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44'),
    ('LICENSE', 'OFL.txt', '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2'),
)


def verified_files():
    for _, name, digest in FILES:
        path = FONT_DIR / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'Run python3 tools/fetch_emoji_font.py: missing or invalid {name}')
        yield path


def main():
    FONT_DIR.mkdir(exist_ok=True)
    for source, name, digest in FILES:
        path = FONT_DIR / name
        if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
            continue
        data = urllib.request.urlopen(SOURCE + source, timeout=60).read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f'SHA256 mismatch: {name}')
        temporary = path.with_suffix('.tmp')
        temporary.write_bytes(data)
        temporary.replace(path)


if __name__ == '__main__':
    main()
