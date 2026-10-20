# 閃 (sen)

閃 is a flash, a fast sword cut along a line: a play on 線 (also *sen*, "line").
The client is meant to be fast.

An unofficial desktop client for the LINE messaging service, using LINE for
Chrome's unmodified crypto WASM and a software-rendered Slint app. Linux is the
first target.

閃 is not affiliated with or endorsed by LY Corporation. LINE is a trademark of
LY Corporation.

Read [HANDOFF.md](HANDOFF.md) before account use. **Chrome owns the secondary-device
slot; native account access requires a new explicit assignment.** Use offline
synthetic demos for development.

## Build and preview

```sh
cargo build --offline --release -p sen-desktop --features demo
target/release/sen --demo
# Headless rendering, with no account access:
target/release/sen --snapshot /tmp/line-preview.ppm
target/release/sen --demo --snapshot-scene photos --snapshot /tmp/line-small.ppm --snapshot-size 360x520 --snapshot-text-size 24
```

The `demo` feature enables synthetic scenarios and snapshots. F12 opens Demo
scenarios; demo sends use a local transport and in-memory SQLite. `--demo-dark`
selects the dark preview palette. Normal builds exclude demo modules and flags.
Snapshots use Slint's software renderer without a desktop window and do not
verify physical input, IME composition or compositor behavior.

## Account setup and storage

Install LINE for Chrome before using an authorized native account. Startup
searches Chrome and Chromium profiles under `$XDG_CONFIG_HOME` (default
`$HOME/.config`) for extension `ophjlpahpchlmihnnnihgmmeilfjmjjc`, choosing the
newest manifest version. It loads `static/js/ltsm.wasm` directly and uses the
manifest version for requests and signing. Required interfaces are checked
before sign-in. No module copy or download script is needed.

With a device-slot assignment, the normal foreground app opens saved state or
offers phone QR/PIN sign-in. Pairing starts only after Start sign-in and can
replace Chrome's login. Text submission requires Send or an enabled shortcut;
Enter inserts a newline by default and Ctrl+Enter sends.

Session credentials live at `$XDG_DATA_HOME/sen/auth/session.json` (default
`~/.local/share/sen/auth/session.json`) and are sealed with the desktop keyring.
`SEN_CREDENTIALS=plaintext-file` explicitly selects the development
credential store; it is not the normal account default. Do not delete keyring
items or existing session files.

History, drafts, preferences and pending submissions live in an unencrypted
SQLite database per account under
`$XDG_DATA_HOME/sen/<mid>/store.sqlite3`. The last account is plain MID text
in `sen/auth/last-account`.
The store opens before network connection when a validated account identity is
available. It retains messages without an age or count limit; the UI subscribes
to windows of that history. Clear only the image cache when removing previews,
not the account database containing drafts and pending sends.

On first start, 閃 copies the old session and last account from `line-desktop`
when their new files are absent. It copies each account store through SQLite
when that account opens, including committed WAL data. Old files and keyring
items are left untouched; caches start fresh. Managed desktop/autostart entries
use the new app id; custom entries are kept.

Image caches use `$XDG_CACHE_HOME/sen/` (default
`~/.cache/sen/`), in `<mid>/` directories, with private directories/files, a 128 MiB / 4096-file
disk LRU and 24 MiB decoded budget. Disk hits refresh mtime; there is no age
expiry. Three asset fetchers download and decode off the UI thread. Originals
and files are requested explicitly. Media authentication precedes decryption.
`--state FILE`, `--wasm FILE`, `--client-version VERSION` and `--cache-dir DIR`
are developer overrides; a module override outside an extension requires a
client version.

RPC, SSE and media use hyper HTTP/1 over async-io with verified rustls/WebPKI
TLS. One shared executor drives connections; RPC, SSE and media have separate
clients/pools. The account net worker owns the WASM runtime and crypto caches;
the engine/store thread owns SQLite. See [the core model](docs/core-model.md).
Live protocol compatibility remains unverified while Chrome owns the slot.

## Install

Build without demo/debug features, then install the executable, icon, launcher
and D-Bus activation service for the current user:

```sh
cargo build --offline --release -p sen-desktop
python3 tools/install-desktop.py
```

The launcher uses XDG defaults. `--destdir DIRECTORY` stages `bin/` and `share/`
without refreshing desktop caches. The installer rejects demo and MCP builds.
The Wayland app ID and X11 class are `org.senclient.Sen`.

## Development checks

```sh
cargo fmt --all -- --check
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
cargo clippy --offline -p sen-desktop --all-targets --features ui-debug -- -D warnings
```

Desktop service contract tests are ignored by default. Run them only on a
private bus:

```sh
dbus-run-session -- env SEN_TEST_BUS=1 cargo test --offline -p sen-desktop -- --ignored
```

MCP is opt-in and absent from normal builds. Build once, then run the checker:

```sh
cargo build --offline --release -p sen-desktop --features ui-debug
python3 tools/check-ui-mcp.py --all
python3 tools/check-ui-mcp.py --group rich --screenshots /tmp/line-ui-shots
python3 tools/check-ui-mcp.py --group layers --wide
python3 tools/check-ui-mcp.py --group signed-out --group missing-module
```

The checker builds nothing and defaults to `target/release/sen`.
It owns offline headless demos, private D-Bus sessions and temporary XDG
directories. `--all` runs the normal and wide matrix plus separate signed-out
and missing-module instances. It prints a seed (`--seed N` reproduces ordering),
per-group results and elapsed time; failures exit nonzero. Synthetic state
resets between groups. Readiness uses MCP initialize retries with a 30-second
deadline. `--screenshots DIRECTORY` saves intermediate and completed surfaces.

For manual inspection, the wrapper owns the same private headless scope:

```sh
python3 tools/run_ui_debug.py --binary target/release/sen --port 9447 -- --demo
```

Ctrl+C, SIGTERM and SIGHUP stop the owned scope and clean temporary directories.
MCP listens only on loopback. Demo exports and attachments stay in the private
`SEN_DEMO_DIRECTORY`. These checks use synthetic accounts, not real messages.
Physical pointer routing, Fcitx composition and desktop interactions require
person-run verification.

## Protocol fixtures and documentation

The root `sen` crate is the default Cargo target. The optional crypto harness
uses synthetic fixtures and discovers the installed module by default. An
explicit WASM argument overrides discovery. AES helpers, Curve25519 key
import/export, runtime inspection and history benchmarks are compiled only with
`harness`; product startup validates only the crypto bindings it calls:

```sh
cargo run --offline --release --features harness --example crypto-harness
cargo run --offline --release --features harness --example crypto-harness -- --bench
```

`tools/generate-fixture.go` and `tools/generate-media-fixture.go` generate
independent synthetic crypto fixtures. `tools/extract_extension.py` can extract
WASM and provenance from an explicitly supplied CRX3 for oracle work.

[docs/issues.md](docs/issues.md) tracks open issues;
[docs/ux-model.md](docs/ux-model.md) defines behavior;
[docs/ux-acceptance.md](docs/ux-acceptance.md) records verification.
[docs/app-design.md](docs/app-design.md) describes engineering constraints and
[docs/chromium-ui-map.md](docs/chromium-ui-map.md) records protocol evidence.
Credentials under `artifacts/auth/`, proprietary artifacts and sensitive HARs
under `dumps/` must stay out of logs, prompts and Git. Checked-in fixtures are
synthetic.

Colour emoji uses Noto Color Emoji (OFL-1.1), installed or downloaded to
`$XDG_DATA_HOME/sen/fonts/` (default `~/.local/share/sen/fonts/`), never embedded.
Privacy: startup contacts jsDelivr for the font and OFL licence when absent,
then checks at most every 30 days using ETag/Last-Modified; failures keep system
fallback or the installed font. Downloads are structurally validated, without a
pinned hash, and registered live. No account information is sent to the CDN.
For development, `python3 tools/fetch_emoji_font.py` (requires python3-fonttools)
fetches the same current bitmap build into gitignored `fonts/`.
`SEN_EMOJI_FONT="$PWD/fonts/NotoColorEmoji.ttf" cargo run -p sen-desktop`
uses that file and disables downloads in debug builds. The UI launcher and
check runner set this override automatically; the installer can copy both files.
See `third_party/README.md` for the renderer and shaping patches.
