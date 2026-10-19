# Native LINE client

A Rust client experiment using LINE Chrome's unmodified crypto WASM and a
software-rendered Slint desktop app. Linux is the first target.

Read [HANDOFF.md](HANDOFF.md) before account use. **Chrome owns the secondary-device
slot; native account access requires a new explicit assignment.** Use offline
synthetic demos for development.

## Build and preview

```sh
cargo build --offline --release -p line-desktop --features demo
target/release/line-desktop --demo
# Headless rendering, with no account access:
target/release/line-desktop --snapshot /tmp/line-preview.ppm
target/release/line-desktop --demo-photos --snapshot /tmp/line-small.ppm --snapshot-size 360x520 --snapshot-text-size 24
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

Session credentials live under `$XDG_DATA_HOME/line-desktop/` (default
`~/.local/share/line-desktop/`) and are sealed with the desktop keyring.
`LINE_DESKTOP_CREDENTIALS=plaintext-file` explicitly selects the development
credential store; it is not the normal account default. Do not delete keyring
items or existing session files.

History, drafts, preferences and pending submissions live in an unencrypted
SQLite database per account under
`$XDG_DATA_HOME/line-desktop/accounts/<own-mid-hash>/store.sqlite3`.
The store opens before network connection when a validated account identity is
available. It retains messages without an age or count limit; the UI subscribes
to windows of that history. Clear only the image cache when removing previews,
not the account database containing drafts and pending sends.

Image caches use `$XDG_CACHE_HOME/line-desktop/` (default
`~/.cache/line-desktop/`), with private directories/files, a 128 MiB / 4096-file
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
cargo build --offline --release -p line-desktop
python3 tools/install-desktop.py
```

The launcher uses XDG defaults. `--destdir DIRECTORY` stages `bin/` and `share/`
without refreshing desktop caches. The installer rejects demo and MCP builds.
The Wayland app ID and X11 class are `org.lineclient.Desktop`.

## Development checks

```sh
cargo fmt --all -- --check
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
cargo clippy --offline -p line-desktop --all-targets --features ui-debug -- -D warnings
```

Desktop service contract tests are ignored by default. Run them only on a
private bus:

```sh
dbus-run-session -- env LINE_DESKTOP_TEST_BUS=1 cargo test --offline -p line-desktop -- --ignored
```

MCP is opt-in and absent from normal builds. Build once, then run the checker:

```sh
cargo build --offline --release -p line-desktop --features ui-debug
python3 tools/check-ui-mcp.py --all
python3 tools/check-ui-mcp.py --group rich --screenshots /tmp/line-ui-shots
python3 tools/check-ui-mcp.py --group layers --wide
python3 tools/check-ui-mcp.py --group signed-out --group missing-module
```

The checker builds nothing and defaults to `target/release/line-desktop`.
It owns offline headless demos, private D-Bus sessions and temporary XDG
directories. `--all` runs the normal and wide matrix plus separate signed-out
and missing-module instances. It prints a seed (`--seed N` reproduces ordering),
per-group results and elapsed time; failures exit nonzero. Synthetic state
resets between groups. Readiness uses MCP initialize retries with a 30-second
deadline. `--screenshots DIRECTORY` saves intermediate and completed surfaces.

For manual inspection, the wrapper owns the same private headless scope:

```sh
python3 tools/run_ui_debug.py --binary target/release/line-desktop --port 9447 -- --demo
```

Ctrl+C, SIGTERM and SIGHUP stop the owned scope and clean temporary directories.
MCP listens only on loopback. Demo exports and attachments stay in the private
`LINE_DEMO_DIRECTORY`. These checks use synthetic accounts, not real messages.
Physical pointer routing, Fcitx composition and desktop interactions require
person-run verification.

## Protocol fixtures and documentation

The root `line` crate is the default Cargo target. The optional crypto harness
uses synthetic fixtures and discovers the installed module by default. An
explicit WASM argument overrides discovery:

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
