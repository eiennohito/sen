# Desktop app sketch

## Product goals

A small, quiet messaging client: fast startup, CPU rendering, low idle memory,
and dependable Japanese input. Slint is the chosen GUI toolkit. Keep the
original LINE crypto blob behind the existing Rust host.

The first window has a searchable conversation list, a message history, and a
multiline composer. Use ordinary text, flat colors, and restrained spacing.
Avoid animated backgrounds, blur, shadows, and continuously running timers.
The default application now opens the saved account, loads real chat metadata,
decrypts recent history, and displays cached assets. Drafts and unresolved
submissions persist in encrypted, account-scoped private state under the XDG
state directory. Explicit legacy/custom session paths retain their configured
state location.
`--demo` and `--snapshot` use synthetic conversations without account access.

## Everyday usability gate

The current sketch is a PoC, not ready for daily use. The user's issue list is
non-exhaustive; audit interaction sequences and asynchronous state changes as
well as appearance. See the reference map's everyday-use requirements for
per-message send/server-confirmation/read states, selection/copy, preview
hierarchy, and content-aware image/sticker backgrounds. A successful send call
is not a read receipt. Implementation and verification status, including partial
behavior and toolkit deviations, is recorded in ux-acceptance.md.

## Rendering and input decision

Pin Slint and slint-build to 1.18.1. Disable default features and enable `std`,
`compat-1-18`, `backend-winit`, `renderer-software`, and `accessibility`.
Select `winit` + `software` programmatically and fail if unavailable. Slint
documents CPU rendering through this backend/renderer combination, and separate
Wayland/X11 feature flags if a smaller platform-specific build is needed.
[Backend docs](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/),
[feature definitions](https://docs.rs/slint/1.18.1/slint/docs/cargo_features/index.html).

Search uses Slint's standard `LineEdit`. The composer wraps Slint's native
multiline `TextInput` in the shared `Pane`: standard `TextEdit` does not expose
the byte-offset selection APIs needed for per-chat caret/selection restoration.
This retains native text editing and IME handling rather than drawing an editor.
Physical Fcitx behavior is still unverified. Unicode font emoji remain
monochrome in the software-renderer captures; LINE image-emoji assets retain
asset colours. Physical desktop emoji rendering is still a V check. Enter inserts a newline or commits
IME input by default; Enter sends is an opt-in preference. Shift+Enter inserts
a newline. Ctrl+Enter uses the same submit action as Send outside composition;
repeat does not resubmit.

The Winit observer tracks native preedit without consuming composition events.
A click on the already-selected chat does not replace editor text. Committed
draft changes go to a coalescing encrypted-storage worker. Switching restores
text, byte-offset caret/selection, and editor scroll; undo-history restoration
is not guaranteed by the native selection API.

Winit-backed Slint has preedit and commit handling; this is a capability, not a
guarantee of perfect IME behavior across input methods and compositors.
The local desktop is Wayland with Fcitx5 available. Treat the checks below as a
release gate. Automated layout snapshots do not satisfy that gate.

## IME acceptance checks

Record compositor, scale factor, input method engine, Slint version, and result.
Test native Wayland first and X11 separately. Run with Japanese fonts installed.

| Scenario | Required behavior | Status |
| --- | --- | --- |
| Type Japanese, convert, select a candidate, press Enter | One committed insertion, no accidental send/newline | Pending |
| Cancel composition with Escape | Preedit cancels; committed draft remains | Pending |
| Reconvert or replace selected text; move within composition | Correct cursor, selection, and surrounding text | Pending |
| Candidate popup near bottom/right edge, at 100%/150%/200% scale | Popup follows caret and stays usable | Pending |
| Focus search, composer, another window, and back mid-composition | No stuck preedit, duplicate text, or loss of committed draft | Pending |
| Click the selected chat during composition | No app-driven editor replacement | Pending |
| Switch conversations and return | Committed drafts stay attached to their conversation | Pending |
| Mixed kana/kanji, Latin, emoji, multiline text, selection, paste, undo | Correct display, editing, and cursor placement | Pending |
| Enter after a trailing newline, scroll long drafts | Last empty line and caret remain visible | Pending |

If a toolkit defect blocks any check, reduce it to a standalone reproduction
before building more messaging features. An alternative Slint backend can be
evaluated then; keep CPU rendering and IME quality as requirements.

## Boundary between GUI and protocol

```mermaid
flowchart LR
    UI[Slint UI thread] -->|bounded commands| Worker[Account worker]
    Worker -->|view updates via event loop| UI
    Worker --> API[LINE JSON / HTTPS]
    Worker --> Crypto[Original WASM via Wasmi]
    Worker --> State[Private session storage]
```

The desktop and CLI now share `src/client.rs` and `src/client/chat.rs`.
`src/client/desktop.rs` owns the account operations, and its `assets` module
owns authenticated media retrieval and private thumbnail caching. The CLI is
a thin frontend over the same transport, session, and WASM implementation.

The GUI thread owns Slint models and text editing. The account worker owns its
WASM runtime, tokens, key caches, RPC and media requests. A separate cancellable
SSE transport thread delivers operations through the bounded worker queue;
desktop D-Bus service/attention work runs outside the UI thread. Do not move a Wasmi store
between threads or perform curl/network work on the GUI thread. Use a bounded
command queue, publish plain view data through `upgrade_in_event_loop`, and
discard a runtime after a WASM exception. Coalesce model updates; never ship
private keys or credentials to the UI. Media keys remain in the worker too.
Late history responses are checked against both chat ID and request generation.
Submission saves a durable pending item before queueing a network command,
then clears the submitted editor. Confirmation updates the item by local ID;
it never clears newer text or changes focus. Known server IDs reconcile to
that same row identity on refresh. Ambiguous sends remain unresolved across
restart and are never retried automatically. Definite rejection and uncertain
results have separate states. Recovery requires a deliberate Replace/Append
choice when newer text exists; Resend warns about duplicate risk.

Each `Api` owns one persistent `ureq::Agent` with rustls and WebPKI roots for
HTTP/1.1 RPCs (no ALPN h2); the agent pool reuses connections. Requests use the
fixed HTTPS gateway, disable redirects (`max_redirects(0)`), and return all HTTP
statuses (including 3xx) without following them. RPCs honour HTTP(S) environment
proxies using curl's HTTPS precedence: `https_proxy`, `HTTPS_PROXY`, then
`all_proxy`/`ALL_PROXY`; `http_proxy`/`HTTP_PROXY` never apply to HTTPS. Empty
values are skipped; a selected invalid or unsupported URL fails with a static
error. `no_proxy`/`NO_PROXY` exclusions apply. CONNECT keeps TLS verified against
the real gateway host. API paths and headers are validated before sending.
Response bodies are capped at 1 MiB while reading,
including chunked and close-delimited bodies. DNS, TCP and TLS share one 10-second
connect budget; each request also has one total deadline covering the body and
any permitted retry.

A custom resolver polls cancellation/deadlines every 50 ms while an OS lookup
runs on a helper thread. It caches successful answers for 30 seconds, including
before pooled requests, and retains at most one outstanding lookup per agent
across cancelled requests. Cache TTL starts when the lookup starts. An abandoned
failure cannot fail a later request, and answers older than one 10-second connect
budget are discarded rather than cached. An OS lookup already running cannot
be interrupted; its receiver is retained so repeated cancellations do not create
more helpers.

A custom TCP connector beneath the stock rustls/CONNECT connectors uses 50 ms
socket read/write timeouts. It retries polling timeouts while checking the
worker stop flag and absolute deadlines, preserving partial writes. Worker
cancellation shuts down the socket and prevents reuse. Sign-in Cancel uses a
separate pairing flag and can wait for the current RPC to finish.

The pool expires idle connections after 15 seconds: a conservative margin below
an assumed gateway keep-alive window of at least 30 seconds, not a live gateway
measurement. A non-blocking socket peek discards EOF or unexpected readable data
before reuse. A connection I/O failure on a reused socket may retry once on a
fresh socket only while the POST is known incomplete. Socket write failures are
recorded below rustls and checked above it after header transmission and before
body transmission, even when rustls accepts plaintext and suppresses the flush
error. A header flush failure therefore returns before body plaintext is offered
and can trigger the one fresh retry. No retry is allowed once the final body
write starts, including an ambiguous TLS flush failure, or after a complete
send. Fresh-connection failures, timeouts and cancellations are not retried. Pooled connections consult the current request's token and deadline.
Transport errors never include tokens, proxy credentials, URLs, or bodies. No
dependency is forked and RPC/SSE HTTPS uses no async runtime. Assets still use
curl.

SSE uses a separate agent with the same proxy selection, verified TLS, resolver
and socket polling as RPCs, keeping its long-lived connection out of the RPC
pool. Each stream attempt forces a fresh connection and closes it on return.
The existing sync thread reads response lines directly, with no curl child,
reader thread or line queue. Status comes from the HTTP response; HTTP 200 opens
the stream regardless of content type, preserving the curl path's behaviour.
HTTP 401/403 retain the existing token-refresh/expiry path. EOF and resets return
immediately to the existing reconnect path. The stop token is passed directly to
the stream request scope and installed for the sync thread's RPCs.

The 30-second ping watchdog counts only time spent waiting in socket reads.
Callbacks, catch-up and queue backpressure do not consume its budget. Only a
decoded ping resets it; other events do not. Once the budget is spent, expiry
is judged before the next socket read, including when a partial line trickles
in. Data already buffered in HTTP or TLS is decoded first. Expiry reports
`SSE ping expired` with the `TimedOut` I/O kind.

Each raw line remains capped at 1 MiB (including its newline), with one extra
byte read to detect overflow; decoder data remains capped at 1 MiB per frame.
The reusable line buffer, bounded decoder event name/data, lossy UTF-8 conversion
and fixed-size HTTP/line-reader buffers replace the former 16-line queue. No
cumulative body cap applies to an indefinitely running stream, but retained
buffering stays bounded even for unterminated lines and multiline frames.
Authentication/signature compatibility remains unverified against an account.
Do not add an async runtime, a browser process, JavaScript, or a database to draw
the window. The sourced SSE path has cancellation, bounded delivery, reconnect
backoff, replay suppression and session fencing. Live protocol compatibility and
refresh behavior still need authorized account verification.

## Memory and responsiveness budget

These are initial engineering targets, not measured claims: an idle desktop
sketch below 80 MiB RSS at 980×720, near-zero idle CPU, and a usable window within
one second of a warm launch. Revisit with measurements on the target desktop.
The integrated account worker needs a separate budget after measuring the real
WASM heap and chat caches.

An initial local Wayland desktop smoke on 2026-10-15, using the release build and
default 980×720 requested size, measured 38,768 KiB RSS (37.9 MiB), 23,867 KiB PSS
(23.3 MiB), and 0.4% CPU over a five-second idle sample after a three-second
settling period. No `/dev/dri` or `/dev/nvidia` file descriptors were open. The
compositor's effective scale factor was not captured, so this is a preliminary
sample, not a cross-machine benchmark. The preview closed cleanly after the
smoke. Japanese composition has not been manually verified.

S10 offline paging measurement (2026-10-17): one release `ui-debug` process,
Slint software renderer on Plasma Wayland at scale 1.5, 980×720 logical pixels
(1470×1080 screenshot). RSS was 62,056 KiB after opening the initial 50-message
page and 71,100 KiB after traversing all 1,000 synthetic messages in 20 pages
(+9,044 KiB). The rendered model stayed at ≤200 rows; retained history stayed
at ≤600 rows per view, with eight total views. This measures that process's
paging growth, not idle CPU, PSS, native-account memory, or physical input.

History retention accounts conservatively for body/metadata, rich/link/reaction
models, and referenced decoded image dimensions. Each retained view and recent
snapshot has a 4 MiB budget; oversized optional image handles are released. One
indivisible oversized text row is retained as an explicit exception, with local
feedback, so budgeting never silently drops the anchored message. Older/newer
range cursors reopen evicted pages through the worker's bounded encrypted cache.
Deferred updates coalesce within bounded chat/image/terminal-event limits;
loss of exact arrival-count precision shows that a refresh is needed.

At 980×720, a single 32-bit pixel buffer costs about 2.7 MiB. More buffers,
HiDPI, font caches, accessibility, WASM memory, and decoded images add to this.
Measure RSS and PSS after first paint and again after opening chats, composing
Japanese, and leaving the app idle. Record window dimensions and scale factor
with each result. A headless snapshot is not an idle desktop memory measurement.

Chats, friends, history and the native composer use the shared `Pane` scrollbar.
Standard Settings, profile and sign-in scrollbars remain documented exceptions;
the viewer uses bounded drag panning. History
exposes at most 200 rows to Slint, with a 600-row / 4 MiB retained budget per
loaded conversation and up to eight cached views. The indivisible-text exception
is described above. Older pages use the
worker's delivered-time/message-ID cursor; window changes preserve a measured
reading anchor. Physical dragging and connected resource measurements remain
release gates.

The worker's decoded thumbnail cache is capped at 24 MiB; private media disk
cache is 128 MiB / 4096 files with 30-day expiry. The decoded message memo is
512 messages / 16 MiB and encrypted recent/range pages share a 32 MiB / 256-file
budget. Network, decryption and decoding remain on workers. The public-key
response cache is bounded to 256 entries.

## Reference audit and caching update

See [chromium-ui-map.md](chromium-ui-map.md) for the 2026-10-15 live Chrome
mapping, coverage matrix, and reconstruction sequence. That sequence takes
priority over expanding behavior without a reference. Friends, bounded paging
and rich-message adapters are now implemented in the worktree; group creation
and management remain outside this plan. Account compatibility is unverified.

Crypto channels now have a 64-entry cache keyed by conversation/key IDs and
group sender. Resetting the WASM also resets channel pointers. Recent-message
pages are encrypted/authenticated with keys derived from the pairing secret;
the disk budget is 32 MiB / 256 files, separate from image-cache limits.
A 512-message / 16 MiB decoded memo fingerprints the raw message, including
reactions. Selection reads the cached page; Refresh explicitly fetches updates.
The SSE adapter supplies live operations and catch-up when authenticated;
offline account startup is still unavailable. Asset arrivals mutate affected
Slint rows while preserving stable row identity. Inline-image messages use
Select text to expose their intact fallback body for selection; cross-run drag
selection and contextual shaping remain Slint 1.18 deviations.

The synthetic release benchmark isolates channel construction: 140.15 ms per
message rebuilding versus 4.54 ms reusing. Connected performance and memory
must be measured after native authorization is deliberately restored. Chrome
currently owns the login; do not run native account checks during the audit.

## Remaining release gates

Use the acceptance rows and session handoffs for the current implementation.
Physical Fcitx/pointer/scale checks and notification/compositor interaction need
a person. Connected sync, token refresh, read markers, media, paging and resource
measurements require an explicit native device-slot assignment. Attachment
sending, group management, animated stickers and reaction editing need separate
protocol/product work. The default close/launch behavior is foreground and quits
unless an explicit background-mode preference is enabled; S12 owns that opt-in.

## Developer UI checks

`--features ui-debug` enables Slint's MCP server and element debug info.
Agent-started previews use `tools/run-ui-debug.py`, which
sets the loopback MCP port inside an owned private D-Bus/virtual KWin scope.
The wrapper maps only its private virtual output and supplies guarded
`--debug-background`; missing prerequisites fail without mapping a foreground
host window. Normal user runs retain foreground startup. Headless snapshots do not
map a GUI window. Regular builds omit this feature, and the desktop installer
rejects debug-server builds. `tools/check-ui-mcp.py` checks an
offline preview, including Japanese/emoji drafts, switching, filtering, and
Enter inserting a line. `--scenarios` also exercises simulated pointer scrolling,
selection/clipboard, a delayed arrival burst, media completion, and failure
recovery. Additional flags cover reading, layers, rich content and paged history;
focused browser/composer scripts test their keyboard and draft invariants.
Run `tools/check-composer-mcp.py --port PORT` against fresh wrapper instances
at both the default `--demo` size (conversation sheet) and
`--demo --demo-size 1400x900` (non-modal panel). `--snapshot-profile` opens a
profile in offline headless snapshots. In the UI, `State.modal-open` includes
the conversation sheet; `State.window-modal-open` covers only the other modal
layers.
The earlier native Wayland check ran at scale factor 1.5. This
does not replace testing preedit and candidate windows with the real IME.

Private state stays out of Git and diagnostics. Before distributing the app,
choose the project's license and the applicable Slint license, and resolve how
users obtain the proprietary crypto artifact; the current repo does not bundle it.


## History

- `7084301`: CPU-rendered Slint desktop sketch and IME-first engineering constraints.
- `6261e8d`: account worker integration, static media caching, desktop identity.
- `6c36f13`: bounded crypto channel reuse, encrypted recent-page cache, Chrome reference map.
- `f7ab75a`: narrow navigation, reading anchors and selectable messages; encrypted
  durable drafts/outbox; explicit authentication recovery; bounded photo viewer.
- `d1fbaa4`: preserve group sender identity through legacy message cache loading.
- `fc4c112`: independent friend/group browsing, profile inspection and local chat navigation.
- `3639708`: modular themed shell, XDG/activation integration, and sourced protocol
  foundations; later session handoffs describe the current integration worktree.

Current behavior is specified by [ux-model.md](ux-model.md); open problems are
registered in [ux-sessions.md](ux-sessions.md) and verification in
[ux-acceptance.md](ux-acceptance.md). History records delivery, not verification
of the remaining physical desktop gates.
