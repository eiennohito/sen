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
between chats to keep separate drafts. Enter inserts a line; Send is a separate
explicit action. Drafts, text size, the selected conversation, and unresolved
submissions survive normal restart in encrypted storage beside the saved session
(`desktop-state/local-state/<pairing hash>/desktop.state`, directories 0700,
file 0600). This storage is separate from the expiring message/image cache.
Do not remove it when clearing thumbnails. A different pairing secret uses a
separate state directory; drafts are never migrated across accounts automatically.
Storage errors remain visible and prevent submission until resolved.

Friends loads your friend list and joined groups independently of conversation
paging. Search loaded names, expand or collapse either category, and open a
profile to inspect its full name and status. **Open chat** reuses an existing
conversation or opens one locally without sending. Browsing and dismissing a
profile retain the current conversation and draft. Refresh failures keep loaded
entries with a local retry explanation. Adding friends, recommendations, group
creation, membership changes, and profile editing remain unavailable.

Preview this flow without accessing the account with `--demo-friends`.
`tools/check-ui-mcp.py --friends` exercises it in a running `--demo` debug UI.

History follows arrivals only at latest. Scrolling or selecting message text
pauses following; Latest resumes it. Reading and editor scroll positions are
kept per chat during the session, with eight bounded cached conversation views.
Message text supports selection, Ctrl+C, and a Copy message context action without displaying an editor caret.
Below 740 logical pixels, Back returns to the list without discarding the open
conversation. Settings → Message text size applies immediately and retains its value.

Refresh retrieves updates; the current history view is bounded to 50 recent
messages per chat. Additional conversation pages can be loaded with More
conversations. Conversations sort by latest activity, with identity preserved
on refresh; messages run oldest to newest, including local pending items. Previews occupy one bounded
line and contact photos use centered circular frames. History paging, unread
tracking, and automatic updates are still unavailable and are not implied by the Latest control.

The default paths are `artifacts/auth/session.json` and
`artifacts/line-3.7.2/ltsm.wasm`, relative to the working directory. Override them
with `--state FILE` and `--wasm FILE`. No new pairing is needed for a valid saved
session. Use `--demo` for the offline synthetic preview. Its History, Arrivals,
Media, and Send state controls exercise long history, a delayed three-message
burst, delayed thumbnail completion, and uncertain submission without account
access. The ordinary Send button remains disabled in this mode.

Install the built executable, original app icon, and desktop launcher for the
current user (the launcher passes absolute session/WASM paths):

```sh
cargo build --release -p line-desktop
python3 tools/install-desktop.py
```

The Wayland app ID and X11 class match `org.lineclient.Desktop.desktop`.
The launcher uses the installed icon's absolute path, and installation refreshes
the icon theme timestamp and available desktop caches so KDE picks up the icon.
The installer refuses builds with the MCP debug feature enabled.

The app explicitly selects Winit with Slint's software renderer. Default Slint
features are disabled; GPU renderers and Qt are not enabled. Both Wayland and
X11 are supported by the chosen backend. Accessibility is enabled. The desktop
compositor may use a GPU; the application renders its own pixels on the CPU.

For a headless rendering of the same window:

```sh
cargo run --release -p line-desktop -- --snapshot /tmp/line-preview.ppm
# Minimum-size conversation at the largest supported text size:
target/release/line-desktop --snapshot /tmp/line-small.ppm --snapshot-size 360x520 --snapshot-text-size 24
```

This renders through Slint's software renderer without creating a desktop window.
It checks layout and text rendering, not desktop input methods or window behavior.
Snapshots always use synthetic conversations, even without `--demo`.

If opening the saved account fails, use **Retry** to reconnect with the saved
session or **Sign in again** to open phone-based QR/PIN sign-in inside the app.
Pairing starts only after **Start sign-in**. It requires `curl` and `qrencode`
and may replace Chrome's active login. Cancel clears the QR/PIN immediately;
an in-flight phone-verification request may need to finish before cancellation
completes. Keep the app open to see success or a retriable error.

A successful sign-in keeps the old session as
`session.before-sign-in-<timestamp>.json`, then installs the new session and
reconnects automatically. Old encrypted drafts remain with the previous pairing;
they are not copied into the new account. QR codes and PINs are not written to
files by native sign-in. A partially completed sign-in may leave a private
`session.sign-in-<timestamp>.json` for recovery; it does not overwrite the old
session before validation succeeds.

## Message cache and performance

Selecting a previously loaded chat uses its encrypted recent-page cache. Click
Refresh to retrieve new messages; automatic updates are not implemented.
The client still needs valid authentication to start, so cached pages do not
provide offline startup. The current native session was invalidated when the
user logged into Chrome; do not re-pair without coordinating that login.

Recent pages use authenticated encryption (AES-256-CTR plus HMAC-SHA256), with
separate keys derived from the saved pairing secret using HKDF. Pages contain
at most 50 messages, have a 32 MiB / 256-file disk budget and 30-day expiry,
and live under the account cache's `messages-v1` directory. A 512-message /
16 MiB decoded memo avoids decrypting unchanged messages during Refresh.
Ciphertext/reaction changes invalidate entries; corrupt pages are discarded.
Clear the account cache to remove both message pages and image previews.

Crypto channels are reused with a 64-entry bound and reset with the WASM runtime.
An offline synthetic benchmark measured 140.15 ms/message rebuilding a channel
versus 4.54 ms reusing one; this is a crypto-stage measurement, not a measured
full-app speedup. Run `cargo run --release --bin line-wasm-poc -- --bench`.

The [Chromium UI map](docs/chromium-ui-map.md) records the live reference,
missing features, rich-message decoding gaps, history protocol, and reconstruction
sequence. The current UI remains an initial sketch.

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

Click a photo preview (sent or received) to load the original in the photo
viewer. Fit, zoom, 1:1, scrolling, and Escape/Close photo are available; closing
restores the conversation and draft. Loading/failure stays in the viewer, with
Retry original on failure. The preview is labeled as such until the original
arrives. Originals are fetched on demand, decoded outside the thumbnail cache,
and released on close; existing 16 MiB encoded / 64 MiB decoded / 8192-pixel
bounds apply. They are not silently replaced by enlarged thumbnails on failure.
Original-file retrieval has source-contract and synthetic coverage; a live
account fetch has not been exercised in the current verification pass.

`--demo-photos` opens a synthetic landscape/portrait fixture. For a minimum-size
layout check without account access:

```sh
target/release/line-desktop --demo-photos --snapshot /tmp/line-photos.ppm --snapshot-size 360x520 --snapshot-text-size 24
```

`--demo-groups` previews sender avatars/names, consecutive-message grouping
with a five-minute boundary, and calendar-day separators. It supports the same
offline snapshot options.

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
python3 tools/check-ui-mcp.py --port 9447 --scenarios --layout-photos --auth-recovery --screenshot /tmp/line-ui.png
```

The server listens on `http://127.0.0.1:9447/mcp`. The feature also enables Slint
element debug information. The UI check refuses a connected-account window,
and exercises chat selection, filtering, draft retention, and multiline editing.
With `--scenarios`, it also dispatches simulated pointer scrolling and keyboard
selection/copy/paste, checking burst counts, reading position, delayed media,
and uncertain-send recovery. These checks do not validate OS pointer routing,
physical scrollbar dragging, or Fcitx composition. Normal installed builds omit
the server.

Remaining limitations: token refresh, automatic live updates, older-history
paging, animated stickers, attachment sending, and reaction editing are not
implemented. Encrypted text sending is wired but has not been verified by an
automated real send. Submission moves text into a durable pending outgoing item and immediately
frees the composer for the next draft. Confirmation updates that item without
implying read. A transport timeout leaves an uncertain outgoing item and never
retries automatically because the message may already have reached LINE.
Append text to draft is a deliberate recovery action; it preserves newer text
and does not submit anything. Confirmation with a known server message ID is
deduplicated on refresh. An uncertain submission without a server ID remains
unresolved; identical text alone is not treated as proof of delivery.

The app architecture and input acceptance checks are described in
[docs/app-design.md](docs/app-design.md). IME acceptance must be verified on the
actual desktop before treating the GUI as ready for messaging.
