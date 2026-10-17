# Native LINE client

A Rust client experiment using LINE Chrome's unmodified crypto WASM, with a
Slint desktop app. Linux desktop is the first target.

The protocol PoC has authenticated a real phone-paired session, restored three
Letter Sealing keys, and decrypted a real V2 chat payload containing a media key.
The desktop app uses the saved session to load real conversation names, recent
messages, photos, stickers, and reactions. Text decryption uses the original
WASM. Text submission is explicit through Send or an enabled send shortcut;
automated validation uses offline synthetic data and does not send account
messages.

The [UX model](docs/ux-model.md) defines the target behavior. See the
[open problems](docs/ux-sessions.md) for the remaining work and
[acceptance record](docs/ux-acceptance.md) for verified coverage. Read
[HANDOFF.md](HANDOFF.md) before account use: Chrome currently owns the device slot.

## Run the desktop app

The normal command opens a foreground window and uses the saved account.
Chrome currently owns its device slot; use `--demo` until native account access
has been explicitly assigned again.

```sh
cargo run --release -p line-desktop
```

Search and select conversations, type into the multiline composer, and switch
between chats to keep separate text, caret, selection and editor scroll state.
Enter inserts a line by default; Ctrl+Enter sends, and Settings has an opt-in
Enter sends preference. Shift+Enter always inserts a line. Drafts, preferences,
the selected conversation and unresolved submissions survive normal restart in
account-scoped encrypted storage under `$XDG_STATE_HOME/line-desktop/local-state/`
(default `~/.local/state/line-desktop/local-state/`, directories 0700, file 0600).
This storage is separate from the expiring message/image cache.
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

History follows arrivals only at latest. User scrolling or selection pauses
following; merely focusing a message does not. Latest resumes following. Reading
and editor scroll positions are
kept per chat during the session, with eight bounded cached conversation views.
Message text supports selection, Ctrl+C, and a Copy message context action without displaying an editor caret.
Below 740 logical pixels, Back returns to the list without discarding the open
conversation. Settings → Message text size applies immediately and retains its value.

The shell has an icon rail, per-destination search, All/Friends/Groups filters,
relative times, outgoing previews, draft/failure markers and keyboard navigation.
Ctrl+1 opens Friends, Ctrl+2 Chats, Ctrl+F focuses browser search, and Alt+Up/Down
switches within the visible conversation list. The open chat and drafts survive
filtering and destination changes. Below 740 logical pixels, a bottom rail and
Back control replace the side-by-side layout.

Desktop notifications and sound default to off; Settings exposes availability,
preview privacy, sound, muted-mention exceptions and group-event preferences.
The default preview is name only. Native desktop notification interaction remains
a physical verification gate; offline automation uses a private bus instead of
the user's notification daemon.

The source includes SSE receive/reconnect, token-refresh, unread/read-range and
older-history adapters. UI history is windowed to 200 rows, with bounded retained
conversation data and encrypted range-page caching. Refresh remains available.
These paths have synthetic coverage; live compatibility and native read markers
remain unverified while Chrome owns the account slot. Unknown mute state is not
shown as a known server preference.

Default session and WASM paths are `$XDG_DATA_HOME/line-desktop/session.json` and
`$XDG_DATA_HOME/line-desktop/ltsm.wasm` (normally under `~/.local/share`). Cache uses
`$XDG_CACHE_HOME/line-desktop`. `--state FILE`, `--wasm FILE` and `--cache-dir DIR`
remain overrides. An explicit custom session path keeps sibling private state
for compatibility. On first launch, existing repository files require an explicit
Use existing files or Start fresh choice; credentials are never copied silently.
`line-auth` shares these session/WASM defaults and saved desktop path choices;
`--state` or `--wasm` uses explicit paths with XDG defaults for omitted paths.
Private desktop state uses `$XDG_STATE_HOME/line-desktop` (normally under
`~/.local/state`). Empty or relative XDG directories fall back to `$HOME`.
Use `--demo` for an offline synthetic preview. Press **F12** in the shell to open
**Demo scenarios**, as indicated in the preview window title. The dialog keeps
scenario controls out of the conversation list and reading surface. Escape or
Close restores the previous focus; selecting a scenario closes the dialog before
running it. Normal account windows have no scenario shortcut. Paging diagnostics
remain in the dialog while the paged-history fixture is active. The ordinary
Send button remains disabled in demo mode.

Install the built executable, original app icon, and desktop launcher for the
current user (the launcher uses XDG defaults; `--dev-paths` explicitly selects
repository development paths):

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
`--demo --snapshot FILE` starts with the demo dialog closed; scenario selectors
such as `--demo-photos`, `--demo-groups` and `--demo-rich` still apply.

If opening the saved account fails, use **Retry** to reconnect with the saved
session or **Sign in again** to open phone-based QR/PIN sign-in inside the app.
Pairing starts only after **Start sign-in**. It requires `curl` and may replace
Chrome's active login. QR encoding runs in Rust, with 10-pixel modules, a
four-module quiet zone and Low error correction. The CLI supports `--qr-file FILE`
for PNG output and `--display ascii|none` (default `ascii` on a TTY, `none`
otherwise). Cancel clears the QR/PIN immediately;
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
Refresh to retrieve an explicit recent-page update; the SSE path handles live
operations when authenticated. The client still needs valid authentication to start, so cached pages do not
provide offline startup. The current native session was invalidated when the
user logged into Chrome; do not re-pair without coordinating that login.

Recent pages use authenticated encryption (AES-256-CTR plus HMAC-SHA256), with
separate keys derived from the saved pairing secret using HKDF. Pages contain
at most 50 messages each, including separately keyed older ranges, with a
32 MiB / 256-file disk budget and 30-day expiry,
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

After explicit native account authorization, a read-only check through the same
shared client reports counts without logging message contents or sending anything:

```sh
target/release/line-desktop --check-live --cache-dir artifacts/cache
```

Click a photo preview (sent or received) to load the original in the photo
viewer. Fit, wheel/keyboard zoom, 1:1, drag pan, Save, previous/next within loaded
photos, and Escape/Close photo are implemented; closing
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
# CLI defaults: $XDG_DATA_HOME/line-desktop/{session.json,ltsm.wasm}
# (fallback $HOME/.local/share/line-desktop/), following the app's saved paths choice.
# To use the repo session explicitly:
target/release/line-auth status --state artifacts/auth/session.json --wasm artifacts/line-3.7.2/ltsm.wasm
target/release/line-auth chat-check --state artifacts/auth/session.json --wasm artifacts/line-3.7.2/ltsm.wasm
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

Slint MCP inspection is opt-in and disabled in normal builds. Agent verification
uses a private virtual compositor so launching a preview cannot take focus from
the user's desktop. Normal app launches remain foreground.

```sh
cargo build --release --offline -p line-desktop --features ui-debug
cp target/release/line-desktop /tmp/line-ui-debug-9447
python3 tools/run-ui-debug.py --binary /tmp/line-ui-debug-9447 --port 9447 --log /tmp/line-ui-9447.log -- --demo
# In another terminal:
python3 tools/check-ui-mcp.py --port 9447 --scenarios --layout-photos --friends --auth-recovery --reading --layers --rich --screenshot /tmp/line-ui.png
```

The wrapper requires `dbus-run-session` and `kwin_wayland`, accepts offline demo
arguments, and supplies the guarded `--debug-background` mode. It refuses a host
window fallback when its private scope cannot start. Use a unique MCP port,
binary copy and log per session; the wrapper prints owned PIDs and a unique PID
file. Stop only that scope. Run `check-browser-mcp.py` and
`check-composer-mcp.py` against fresh owned previews; the composer check quits
its preview. `--paging` adds the bounded 1,000-message history scenario.

The server listens on `http://127.0.0.1:9447/mcp`. The feature also enables Slint
element debug information. The UI check refuses a connected-account window,
and exercises chat selection, filtering, draft retention, and multiline editing.
With `--scenarios`, it also dispatches simulated pointer scrolling and keyboard
selection/copy/paste, checking burst counts, reading position, delayed media,
and uncertain-send recovery. These checks do not validate OS pointer routing,
physical scrollbar dragging, or Fcitx composition. Normal installed builds omit
the server.

Remaining limits and deviations are tracked in the acceptance record: physical
IME/compositor checks, live account compatibility, native mute sourcing,
attachment sending, animated stickers and reaction editing remain incomplete or
unverified. Rich messages retain intact fallback-body copy/selection through
Select text; cross-run drag selection and contextual shaping are toolkit limits.

Submission creates a durable pending item and frees the composer immediately.
Sent does not imply Read. Definite rejection offers Retry and Edit; an uncertain
result requires an explicit duplicate-risk confirmation before Resend. Edit
requires a Replace/Append decision when a newer draft exists. Request identities
and known server IDs reconcile confirmations without using identical text as
proof of delivery. Physical Japanese composition remains a release gate.

The app architecture and input acceptance checks are described in
[docs/app-design.md](docs/app-design.md). IME acceptance must be verified on the
actual desktop before treating the GUI as ready for messaging.
