# Native LINE client

A Rust client experiment using LINE Chrome's unmodified crypto WASM, with a
Slint desktop app. Linux desktop is the first target.

The protocol PoC has authenticated a real phone-paired session, restored three
Letter Sealing keys, and decrypted a real V2 chat payload containing a media key.
The desktop app uses the saved session to load real conversation names, recent
messages, photos, stickers, and reactions. Text decryption uses the original
WASM. Send encrypts text and submits it only when the user clicks the button;
automated validation does not send messages.

## Run the desktop app

```sh
cargo run --release -p line-desktop
```

Search and select conversations, type into the multiline composer, and switch
between chats to keep separate drafts in memory. Enter inserts a line; Send is
a separate action. Drafts are discarded when the app closes. Refresh retrieves
updates; the current history view is bounded to 50 recent messages per chat.
Additional conversation pages can be loaded with More conversations.

The default paths are `artifacts/auth/session.json` and
`artifacts/line-3.7.2/ltsm.wasm`, relative to the working directory. Override them
with `--state FILE` and `--wasm FILE`. No new pairing is needed for a valid saved
session. Use `--demo` for the offline synthetic preview.

Install the built executable, original app icon, and desktop launcher for the
current user (the launcher passes absolute session/WASM paths):

```sh
cargo build --release -p line-desktop
python3 tools/install-desktop.py
```

The Wayland app ID and X11 class match `org.lineclient.Desktop.desktop`.
The installer refuses builds with the MCP debug feature enabled.

The app explicitly selects Winit with Slint's software renderer. Default Slint
features are disabled; GPU renderers and Qt are not enabled. Both Wayland and
X11 are supported by the chosen backend. Accessibility is enabled. The desktop
compositor may use a GPU; the application renders its own pixels on the CPU.

For a headless rendering of the same window:

```sh
cargo run --release -p line-desktop -- --snapshot /tmp/line-preview.ppm
```

This renders through Slint's software renderer without creating a desktop window.
It checks layout and text rendering, not desktop input methods or window behavior.
Snapshots always use synthetic conversations, even without `--demo`.

## Image cache

Avatars, static sticker frames, photo previews, and paid-reaction images use an
account-scoped cache under `$XDG_CACHE_HOME/line-desktop` (default
`~/.cache/line-desktop`). Use `--cache-dir DIR` to override it. Cache directories
are mode 0700 and files are 0600. Filenames are hashes; URLs, tokens, and media
keys are not written to cache metadata. Decrypted thumbnail pixels are private
account data, so clearing this directory removes cached previews.

Encoded thumbnails have a 128 MiB / 4096-file disk limit and expire after 30
days. The decoded worker cache is bounded to 24 MiB. Downloads and decoding run
off the UI thread, with size limits; images are reduced before reaching Slint.
Standard reactions use emoji, while paid reactions use cached static images.
Stickers and reactions do not animate yet. Media authentication is checked
before decryption; photos use the encrypted-media token and Chrome's file
format, which lives outside the LTSM blob in the original extension too.

A read-only check through the same shared client reports counts without logging
message contents or sending anything:

```sh
target/release/line-desktop --check-live --cache-dir artifacts/cache
```

## Protocol tools

The root package remains the default Cargo target, so ordinary CLI builds do not
compile the GUI. Obtain the official extension WASM separately; see
[HANDOFF.md](HANDOFF.md) for its supported version, hash, state handling, and
verified protocol details. The blob is not included in Git.

```sh
cargo build --release --bins
target/release/line-auth status
target/release/line-auth chat-check
cargo run --release --bin line-wasm-poc
```

`login` performs QR pairing when needed; preserve an existing session. `status`
checks saved authentication and key restoration. `chat-check` performs a bounded
read-only decryption probe and reports counts without saving message contents.

Credentials and wrapped keys under `artifacts/auth/`, proprietary extension
artifacts, and sensitive HAR captures under `dumps/` are ignored by Git. The
checked-in crypto fixture is synthetic. This is a local repository; no remote is
configured.

## Development

```sh
cargo fmt --all --check
cargo test --release --workspace
cargo clippy --release --workspace --all-targets -- -D warnings
```

Slint MCP inspection is opt-in and disabled in normal builds:

```sh
cargo run --release -p line-desktop --features ui-debug -- --demo
# To start the MCP server, set this environment variable before launching:
SLINT_MCP_PORT=9447 cargo run --release -p line-desktop --features ui-debug -- --demo
python3 tools/check-ui-mcp.py --port 9447 --screenshot /tmp/line-ui.png
```

The server listens on `http://127.0.0.1:9447/mcp`. The feature also enables Slint
element debug information. The UI check refuses a connected-account window,
and exercises chat selection, filtering, draft retention, and multiline editing.
It does not validate Fcitx composition. Normal installed builds omit the server.

Remaining limitations: token refresh, automatic live updates, older-history
paging, animated stickers, attachment sending, and reaction editing are not
implemented. Encrypted text sending is wired but has not been verified by an
automated real send. A transport timeout preserves the draft and does not retry
automatically because the message may already have reached LINE.

The app architecture and input acceptance checks are described in
[docs/app-design.md](docs/app-design.md). IME acceptance must be verified on the
actual desktop before treating the GUI as ready for messaging.
