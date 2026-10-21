# Colour emoji patches

Vendored Slint software renderer and parley crates, patched so colour emoji
render. Local changes against `dac76c7` (pristine crates.io sources):

- `i-slint-renderer-software` 1.18.1: render COLRv0 and colour bitmap glyphs as
  premultiplied RGBA, preserving text alpha and COLR foreground RGB. Fonts with
  colour bitmaps use subpixel bin 0 for every glyph; fonts with COLR key on
  foreground RGB.
  The alpha-only glyph path skips colour fonts; text strokes skip colour images.
- `parley` 0.11.1: try the Emoji family first for emoji-presentation clusters;
  retain text-first fallback for text presentation, including FE0E.

From the repository root, run:

```sh
cargo test --offline -p i-slint-renderer-software
cargo test --offline --manifest-path third_party/parley-emoji-tests/Cargo.toml --target-dir target
```

The first command runs the renderer's unit and documentation tests, including
COLRv0 foreground/alpha and sbix edge premultiplication regressions. Its generated
fixtures and fontTools scripts are in `i-slint-renderer-software-1.18.1/tests/fonts/`.

The Parley wrapper runs `parley-0.11.1/tests/emoji.rs` against the patched crate.
The published crate lacks the `parley_dev` dependency needed by its upstream unit
tests. The integration test loads checked-in Noto Color Emoji and DejaVu Sans
subsets without system fonts or environment variables. Regenerate them with fontTools:

```sh
python3 third_party/parley-0.11.1/tests/fonts/subset.py \
  /path/to/DejaVuSans.ttf
```

The script verifies the local emoji input against `tools/fetch_emoji_font.py` and
pins DejaVu Sans 2.37 by SHA256. It downloads nothing. The text fixture contains
only DejaVu Sans glyphs; its licence and the emoji OFL accompany the subsets.
Skin-tone and tag-flag cases are omitted because the text font lacks their bases.
Rebase these patches when upgrading Slint.
