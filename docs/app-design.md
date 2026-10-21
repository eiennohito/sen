# Desktop app design

Engineering constraints for the 閃 desktop app: toolkit, input, workers,
transport, errors and performance budgets. Behaviour is in
[ux-model.md](ux-model.md); crates, threads and the store are in
[core-model.md](core-model.md).

## Product goals

A small, quiet messaging client: fast startup, CPU rendering, low idle memory,
and dependable Japanese input. Slint is the GUI toolkit; LINE's original
crypto module runs behind the Rust host. Use ordinary text, flat colours and
restrained spacing. Avoid animated backgrounds, blur, shadows and continuously
running timers. The app opens the account's SQLite store before connecting, so
cached chats are usable offline. `--demo` and `--snapshot` use synthetic
conversations without account access.

## No compatibility code

Nothing outside this repository calls the code, so there is nothing to stay
compatible with. When something is renamed, moved or redesigned, every caller
changes in the same commit. No aliases, re-exports, forwarding `<=>`
properties, shims or "preserves the old API" layers. Persisted user data is
the exception: the saved session must keep loading, and the local store is
migrated rather than discarded (see [deslop.md](deslop.md) and
[core-model.md](core-model.md#store)).

## Rendering and input

Slint and slint-build are pinned to 1.18.1 with default features off and
`std`, `backend-winit`, `renderer-software` and `accessibility` enabled.
Normal launches select `winit` + `software` programmatically and fail if they
are unavailable.

Search fields and the composer wrap Slint's native `TextInput`, keeping native
caret handling, selection, clipboard and IME. Search adds its own Cut, Copy,
Paste and Select All menu and a 32 px Clear button. The composer uses
`TextInput` rather than `TextEdit` because only `TextInput` exposes the
byte-offset selection API needed to restore each chat's caret and selection.
Switching chats restores text, caret/selection and editor scroll; undo history
is not restored.

Text entry uses Slint's IME-aware components as they are: no custom preedit
handling and no key interception during composition. The one exception is the
winit IME observer behind `ShellState.ime-active`: Slint 1.18.1 calls the
app's `key-pressed` callback before its own preedit check, so the observer
tracks whether a preedit is active and keeps app shortcuts, including
Enter-to-send, from running during composition. It propagates every event.

Enter inserts a newline or commits IME input by default; "Enter sends" is an
opt-in preference. Shift+Enter always inserts a newline. Ctrl+Enter sends
outside composition; key repeat does not resubmit.

## Manual IME checks

Automated snapshots cannot check IME behaviour. Run these by hand with
Japanese fonts installed, native Wayland first and X11 separately, and record
compositor, scale factor, input method, Slint version and result.

| Scenario | Required behaviour |
| --- | --- |
| Type Japanese, convert, select a candidate, press Enter | One committed insertion, no accidental send or newline |
| Cancel composition with Escape | Preedit cancels; committed draft remains |
| Reconvert or replace selected text; move within composition | Correct cursor, selection and surrounding text |
| Candidate popup near the bottom/right edge, at 100/150/200 % scale | Popup follows the caret and stays usable |
| Focus search, composer, another window and back mid-composition | No stuck preedit, duplicate text or lost draft |
| Click the selected chat during composition | No app-driven editor replacement |
| Switch conversations and return | Committed drafts stay with their conversation |
| Mixed kana/kanji, Latin, emoji, multiline text, selection, paste, undo | Correct display, editing and cursor placement |
| Enter after a trailing newline, scroll long drafts | Last empty line and caret remain visible |

If a toolkit defect blocks a check, reduce it to a standalone reproduction
before building more on top of it.

## Crypto module

The crypto module comes from the installed LINE for Chrome extension
(`ophjlpahpchlmihnnnihgmmeilfjmjjc`). Startup searches Chrome and Chromium
profiles under `$XDG_CONFIG_HOME` (default `$HOME/.config`) and selects the
newest manifest version across all profiles; the module is
`static/js/ltsm.wasm` under that extension root. The manifest's `version` is
used for signing, headers and receive URLs. `--wasm` and `--client-version`
are developer overrides.

Right after instantiation the host probes the required embind classes,
constructors, methods and functions. A missing one gives "LINE for Chrome
<ver> changed its crypto module (missing <name>); this app doesn't support it
yet" on sign-in, in the connection strip and in the log. No extension gives
"Install LINE for Chrome — this app uses its crypto module" with the searched
locations. Sessions do not depend on the module build. See
[core-model.md](core-model.md#crypto-boundary) for runtime ownership.

## Workers

The UI thread owns Slint models and text editing and never does network,
decryption or decoding work. Private keys, tokens and media keys stay on
background workers. Never move a wasmi store between threads.

The stream thread borrows authorisation from the net thread and asks it to
refresh tokens; it never instantiates WASM. Missing device keys, RPC errors
and parse errors fail only their item. The latest peer and group keys are
fetched per send; key-ID data and channels are cached.

Three asset threads share one asset HTTP client and the thumbnail cache. Three
slots let visible images overlap while bounding simultaneous encoded
downloads to 48 MiB (16 MiB each). Cache misses ask the net thread for media
authorisation; cache hits need no token RPC. The queue holds at most 256
distinct pending or active keys and de-duplicates in-flight requests; explicit
requests (originals, files, item Retry) come before thumbnails, FIFO within
each class, and overflow evicts the oldest pending thumbnail. Failures keep
their class; retriable ones become eligible again after 300 seconds, and item
Retry bypasses both the retained failure and the cache. Reconnect, sign-in and
shutdown stop and join the fetchers before credentials change. The disk cache
is an LRU by mtime: 128 MiB / 4096 files for thumbnails and 32 MiB / 128 files
for originals.

Submission saves a durable pending row before queueing the network command,
then clears the editor. Confirmation links the server ID to the same row key;
it never clears newer text or moves focus. Failed sends show "Not sent:
<reason>" with Retry and Delete, including rows that were in flight when the
app closed; they are never resent automatically. Our own echo (operation 25 or
43 with our `reqSeq`) confirms a row in any state as Sent.

## Transport

RPC, SSE and media use separate hyper 1 HTTP/1 clients with verified rustls
(ring, WebPKI roots). One process-wide executor thread drives connections;
blocking callers wait on request futures. Each client keeps at most one idle
connection per host for 15 seconds. Only a request that hyper returns unsent
from a stale idle connection is retried, once. Requests returned unsent are
NotDelivered; requests that may have been sent are Unanswered.

Production URLs must use HTTPS. DNS, TCP, proxy CONNECT and TLS share a
10-second connect budget (5 seconds for assets). RPC and media GET have total
deadlines (media GET 15 seconds). Uploads have no total cap; 60 seconds
without progress expires them. Cancellation races requests and body reads and
drops the connection. Errors never include tokens, proxy credentials, URLs or
bodies. Redirects are not followed.

Public assets carry no RPC or private media headers. The asset host allow-list
and the 16 MiB download cap apply before decoding. Proxy selection uses
`https_proxy`/`HTTPS_PROXY`, then `all_proxy`/`ALL_PROXY`, with
`no_proxy`/`NO_PROXY` exclusions (exact host, domain suffix, `*`). HTTP CONNECT
supports Basic credentials. An applicable proxy with any scheme other than
`http` is a configuration error and is never bypassed.

SSE uses a fresh connection. HTTP 200 opens the stream regardless of content
type; 401/403 enter the token refresh path. The ping watchdog (30 seconds)
counts only time spent waiting for body bytes, and only a decoded ping resets
it; expiry or a read error restarts the stream. Raw lines and decoder frames
are capped at 1 MiB each, so buffering stays bounded even for unterminated
lines. Do not add an async runtime, a browser process or JavaScript.

## Error reporting

Every message the user sees says what failed and what happens next or what
they can do, at the place of the thing that failed (ux-model §9.1). There is
no catch-all banner.

- **Classify where the error happens.** The engine reports typed errors, not
  strings, and the presenter maps each kind to its own text:
  - **A failed user action** (send, unsend, reaction, rename, add friend,
    create group, leave) fails on that item: "Couldn't add friend."
  - **Connection and authentication** go to the connection strip.
  - **A real local write failure** (SQLite I/O, disk full, permission) names
    the cause and the consequence, for example "Can't save messages: disk is
    full. New messages are shown but won't be kept." It shows under the
    composer and in Settings and clears on the next successful write.
  - **Background refreshes** reschedule network failures quietly; Refused and
    Bug stop the item.
- **Bugs are not user errors.** Invariant violations are logged with the
  intent and are never shown as a storage problem. `ui-debug` builds show
  them in a debug-only toast so checks notice them.
- **Nothing runs that can't work.** When signed out, the engine doesn't run
  account operations at all.
- **One log.** Errors go to stderr (the journal under systemd) with a
  timestamp, the operation, the error kind and its chain of categories. Logs
  never contain tokens, message text, names or MIDs.

## Text layout

Chats use a `ListView`. History uses one when the initial window has more than
60 rows and keeps that mode while the conversation is open, so arrivals never
swap delegates at the cutoff. Asset arrivals patch the affected rows and
nested runs in place, keeping row identity.

Messages with inline images or links render as one measured flow broken at
Unicode line-break opportunities, with URL delimiters as extra breaks; only an
oversized unbreakable span falls back to grapheme breaks. Runs are measured
at 64 times the font size, so rounding stays within 1/64 px. Each window has a
measurement LRU keyed by text and font size: at most 4096 entries and 1 MiB of
strings. Measurement and packing finish synchronously before painting, with
no remeasurement timer. Messages over 2048 runs fall back to the plain
selectable body. Rich flows select inline: drag selects across runs,
double-click selects a word, and a click on a link opens it.

Chat-list previews carry the same typed message, emoji keys and media
thumbnail keys as history, but render a measured line only for rows in the
viewport. Emoji text for accessibility, copy, notifications and search stays
the original placeholder (for example `[笑]`). Missing emoji keep a reserved
box, so loading never reflows the conversation.

## Performance budgets

閃 is meant to be fast. These budgets apply to release `ui-debug` builds on
the development machine; exceeding one is a regression like a failing test.

| What | Budget |
| --- | --- |
| Chat switch, Rust + layout + draw | ≤ 16 ms, one 60 Hz frame |
| One asset arriving, Rust + layout + draw | ≤ 4 ms |
| 100 avatars arriving at once | ≤ 100 ms |
| Steady-state frame, median | ≤ 4 ms |
| First frame of a scene (chats, photos, rich) | ≤ 16 ms |
| Rich message drag, update + draw (10 lines) | ≤ 4 ms |
| Cached image | visible on the first frame its row is shown |
| Opening a 12 MP photo in the viewer | ≤ 150 ms |
| Idle main-thread wakeups | ≤ 10 /s |
| Idle memory at 980×720 | below 80 MiB RSS, near-zero CPU |

Measure with the latency harness and the `IDLE` and frame lines of
`tools/check-ui-mcp.py --all` on a release build. The harness
(`app/src/dev/latency.rs`) builds 300 chats, a 500-message window and 100
distinct avatars, and reports medians of repeated asset arrivals, chat
switches and rich-selection drags with the software renderer:

```sh
cargo build --release --offline -p sen-desktop --features ui-debug
SEN_UI_LATENCY=1 target/release/sen --demo --snapshot /tmp/latency.ppm
```

Measure memory after first paint and again after opening chats, composing
Japanese and idling, with window size and scale factor. A headless snapshot is
not an idle desktop measurement.

## Developer builds

See [build-iteration.md](build-iteration.md) for the `fast` profile and the
UI checker, and the README for the full command list.

`--features ui-debug` enables Slint's MCP server and element debug info and
implies `demo`; normal builds omit both, and the installer rejects such
builds. `tools/run_ui_debug.py` starts a headless instance with the MCP port
inside a private D-Bus session and private XDG directories, and supplies
`--debug-background`. Demo and snapshot scenes use `FakeBackend` through the
same engine, in-memory SQLite store, controller and presenter. F12 opens the
demo scenarios (arrivals, media failures, rejected and unanswered sends,
reading and rich-content fixtures, attachments). Snapshot scenes are defined
in `app/src/dev/demo.rs`; `--snapshot-size`, `--snapshot-text-size`,
`--snapshot-profile` and `--demo-dark` combine with them. Headless snapshots
fix the clock at 2026-10-18 14:00 local time.
