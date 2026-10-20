# 閃 (sen)

![閃 icon](app/assets/icon.svg)

An unofficial, fast desktop client for the [LINE](https://line.me) messenger on
Linux, written in Rust with [Slint](https://slint.dev).

閃 (*sen*) is a flash, a fast sword cut: a play on 線 (also *sen*, "line").

## tl;dr

閃 is a native LINE client for Linux desktops. It speaks the same protocol as
the official LINE for Chrome extension and borrows that extension's crypto
module from your browser, so you need the extension installed. Sign in by
scanning a QR code with LINE on your phone.

```sh
# Needs: a recent stable Rust, a C compiler, pkg-config, fontconfig headers,
# a Secret Service keyring (GNOME Keyring, KWallet, …) and LINE for Chrome
# installed in Chrome or Chromium.
git clone https://github.com/eiennohito/sen.git && cd sen
cargo build --release -p sen-desktop
python3 tools/install-desktop.py   # binary, icon, launcher, D-Bus service for your user
sen                                 # or just run: cargo run --release -p sen-desktop
```

Builds are local only for now: no CI, no packages.

**Signing in to 閃 uses the same device slot as LINE for Chrome, so it signs
the extension out** (and the other way round). 閃 is not affiliated with or
endorsed by LY Corporation, and using an unofficial client may be against
LINE's terms of service. Use it at your own risk.

## This project is vibe coded

The whole project is vibe coded: AI agents wrote the code, tests and docs. I
directed them, tested the app and fixed what was broken, but I mostly did not
read the source, apart from the worst problems. Expect it to look like that in
places. Bug reports with steps to reproduce are welcome.

## What works

- Chats, groups and friends, with search across kana and romaji.
- Text, replies, mentions, LINE emoji, stickers, photos, files and Flex
  (rich) messages. Rich text is selectable.
- Sending text, stickers, photos and files, including paste and drag and drop.
- Reactions (set, remove, see who reacted), read receipts and unread counts.
- Editing your own display name and status message.
- Desktop notifications, tray icon, background mode and autostart.
- Logging out, optionally deleting that account's local data.
- History is kept locally, so cached chats open instantly and offline.
- Follows the system light or dark theme; full keyboard navigation and screen
  reader labels.

Not supported (yet): calls, blocking and reporting, group invitations and
member management. Unsupported message types show as "This message type isn't
supported yet". [docs/issues.md](docs/issues.md) tracks everything open.

## How it works

LINE for Chrome talks JSON over HTTPS to LINE's Chrome gateway and does
end-to-end encryption (Letter Sealing) in a WebAssembly module. 閃 implements
that protocol in Rust and runs the extension's unmodified `ltsm.wasm` with
[wasmi](https://github.com/wasmi-labs/wasmi) for the crypto. The module is
never copied or redistributed. At startup 閃 looks for the newest installed
extension (`ophjlpahpchlmihnnnihgmmeilfjmjjc`) in Chrome and Chromium profiles
under `$XDG_CONFIG_HOME` (default `~/.config`), loads `static/js/ltsm.wasm` and
uses the extension's version for requests. Reaction artwork is also loaded from
the installed extension.

The UI is Slint with the software renderer on winit (Wayland and X11). HTTP is
hyper with rustls and WebPKI roots. SQLite holds the local history. See
[docs/core-model.md](docs/core-model.md) and [HANDOFF.md](HANDOFF.md) for the
architecture.

## Data and privacy

- **Session credentials**: `$XDG_DATA_HOME/sen/auth/session.json` (default
  `~/.local/share/sen/`), sealed with a key kept in your desktop keyring
  (`application=sen`). Deleting that keyring item signs you out.
- **History, drafts, preferences and the outbox**: one unencrypted SQLite
  database per account at `$XDG_DATA_HOME/sen/<mid>/store.sqlite3`, with no age
  or size limit. Store format changes are migrated, never wiped.
- **Image cache**: `$XDG_CACHE_HOME/sen/<mid>/` (default `~/.cache/sen/`),
  private files, 128 MiB LRU. It is safe to delete.
- **Emoji font**: colour emoji use Noto Color Emoji (OFL-1.1). If your system
  doesn't have a usable copy, 閃 downloads it once from jsDelivr into
  `$XDG_DATA_HOME/sen/fonts/` and checks for updates at most every 30 days. No
  account information is sent. Without it, emoji fall back to system fonts.

閃 talks only to LINE's servers and, for the font, to jsDelivr.

## Install details

`tools/install-desktop.py` installs a plain release build (it refuses demo and
debug builds) to XDG locations for the current user: the `sen` binary, the
icon, a `.desktop` launcher and a D-Bus activation service. `--destdir DIR`
stages `bin/` and `share/` instead, without refreshing desktop caches. The
Wayland app id and X11 class are `org.senclient.Sen`.

## Development

Agents and contributors follow [docs/deslop.md](docs/deslop.md) and
[docs/dependencies.md](docs/dependencies.md).
[docs/ux-model.md](docs/ux-model.md) defines behaviour,
[docs/visual-design.md](docs/visual-design.md) the look, and
[docs/protocol-traces.md](docs/protocol-traces.md) the protocol evidence.

Synthetic demo (no account, no network):

```sh
cargo run --release -p sen-desktop --features demo -- --demo
```

F12 opens demo scenarios and `--demo-dark` selects the dark palette.
`--snapshot FILE` renders headless, for example
`--demo --snapshot-scene photos --snapshot shot.ppm --snapshot-size 360x520`.

Checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p sen-desktop --all-targets --features ui-debug -- -D warnings
cargo test --workspace
```

Desktop service tests are ignored by default. Run them only on a private bus:

```sh
dbus-run-session -- env SEN_TEST_BUS=1 cargo test -p sen-desktop -- --ignored
```

The UI checker drives a headless `ui-debug` build (MCP on loopback, synthetic
accounts, private D-Bus and XDG directories) and builds nothing itself:

```sh
cargo build --profile fast -p sen-desktop --features ui-debug
python3 tools/check-ui-mcp.py --all --binary target/fast/sen
python3 tools/check-ui-mcp.py --group rich --screenshots target/shots --binary target/fast/sen
```

It prints a seed (`--seed N` reproduces the order) and per-group results.
`python3 tools/run_ui_debug.py --binary target/fast/sen --port 9447 -- --demo`
starts the same private headless instance for manual inspection.

For emoji in development builds, `python3 tools/fetch_emoji_font.py` (needs
python3-fonttools) fetches the font into the gitignored `fonts/`, and
`SEN_EMOJI_FONT="$PWD/fonts/NotoColorEmoji.ttf"` uses it without downloading.
The checker and launcher set this automatically.

The crypto harness tests the module against synthetic fixtures:

```sh
cargo run --release --features harness --example crypto-harness
```

Never commit credentials, captures or HAR files: `artifacts/` and `dumps/` are
gitignored for that. Checked-in fixtures are synthetic.

## Licence

閃 is licensed under the [GNU GPL v3](LICENSE) (`GPL-3.0-only`), in line with
Slint's GPL option. Vendored and patched crates keep their own licences: Slint's
software renderer (GPL-3.0-only among its options) and parley (Apache-2.0 or
MIT) under [third_party/](third_party/README.md). The WASM binding conventions
are informed by beeper/line (MIT, [licence](licenses/beeper-line-MIT.txt)).
Noto Color Emoji is OFL-1.1 and is downloaded, never bundled.

LINE is a trademark of LY Corporation. 閃 is not affiliated with or endorsed by
LY Corporation.
