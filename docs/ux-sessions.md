# Open problems

Written 2026-10-17 from an audit of commit `05cd26d`: headless snapshots at
980×720, 1400×900, 760×600 and 360×520 (15 and 24 px), a driven run on a
private virtual KWin through the Slint MCP server, and source reading. The
behaviour to build is still [ux-model.md](ux-model.md). The previous session
plan and its hand-offs are in the git history of this file; they described
work as done that this list shows is not.

Rules that still apply: Slint 1.18.1 pinned, software renderer, no async
runtime, worker threads own network and crypto, UI thread owns models, no
account access without the device slot. Agent-driven previews go through
`tools/run-ui-debug.py`.

Order of work: T first, because every "slow" item below it is partly T. Then
P, then the UI groups in any order. V closes the loop.

## T — Transport and worker

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| T5 | Reaction counts never update live: the sync decoder handles no reaction operation; counts change only when a message page is refetched. | `src/client/desktop/sync.rs` (no match for reaction) | Decode the reaction operation into an in-place row update; it is a quiet event in the attention matrix. |
| T9 | Crypto runs the Chrome WASM blob in the `wasmi` interpreter for every encrypt and decrypt; the sync thread instantiates a second runtime; the user must supply `ltsm.wasm`. | `src/runtime.rs:506` | Trial done: `src/native_crypto.rs` ports derivation, HMAC, AES-GCM, Letter Sealing V1/V2 and X25519, proven byte-for-byte against the blob (see [crypto-port.md](crypto-port.md)); about 11,000× faster on a V2 decrypt. Blocked: `SecureKey` (SKB) keys are opaque, so stored-key loading, token recovery and MID decoding still need the blob. Next: decide whether to switch message crypto to native while keeping wasmi for key unwrapping. |
| T11 | Session credentials (refresh token, wrapped keys) are plaintext JSON at mode 0600; drafts are encrypted, credentials are not. | `src/client.rs:88`, usage text at `:771` | Same encrypted store as drafts, or the desktop secret service. |

## P — Protocol gaps

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| P1 | Official-account media uses a different route. The client only builds `obs.line-apps.com/r/talk/{SID}/{OID}/preview` with defaults `m` and the message id, so OA images 404 and show as "Preview unavailable". The map records that `DOWNLOAD_URL` variants are refused on purpose. No OA message exists in `dumps/`. | `src/client/desktop/assets.rs:106`, `docs/chromium-ui-map.md:351` | Trace one OA image message (its `contentMetadata` keys and resolved host), add that host to the allowlist, build the right URL. Interim wording "Not available for this sender" (any image 404) landed with T4. |
| P2 | Sticker sending does not exist: no picker, no send path. Listed as "later". For a LINE client it is a daily-use blocker. | `app/ui/composer.slint`, `app/src/composer.rs` | Trace one sticker send (STKID/STKPKGID/STKVER shape is in the HAR), implement the send, then a picker as a panel in wide and a sheet in narrow. |
| P3 | Attachment sending, paste and drop of media do not exist (CMP8 media, CMP9, OS10). | composer | After P2; needs the upload trace. |
| P4 | Link detection only matches `https://`, `http://`, `www.`. Bare domains, uppercase schemes and links split across emoji runs are missed. | `app/src/links.rs:13` | Broaden the matcher; run it over the rendered text, not the pre-run body. |
| P5 | Group members cannot be seen, and nothing that depends on them exists: no members panel from the group header, no member profile from it, no member names/avatars for senders who are not friends beyond what message rows carry, no pending invitations to accept or reject, no invite, remove, leave, or group name/image edit. The data is partly there: `getChats` is already called with `withMembers` and `group_member_count` reads `memberMids`, but only a count reaches the UI. Chrome has a group header member button and a separate member window (`docs/chromium-ui-map.md:71`, `:73`); the API list is at `:235`. | `src/client/desktop.rs:314`, `:426`; `src/client/desktop/rich.rs:146`; `app/ui/conversation.slint` header | Read-only first: members panel (model: Panel; sheet when narrow) opened from the group header title/member count, listing members with name, avatar and self, searchable, each opening the existing profile; non-friend names resolved through the contact lookup the client already uses, unknown shown as such. Then invitations (accept/reject in the chat list) and invite/remove/leave/rename, each needing a trace and confirmation design; mutating calls cannot be verified without the device slot. |

## S — Shell and layout

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| S5 | Chat-row hover is invisible: hover colour equals the pane background. | `app/ui/chats.slint:83` vs `app/ui/app.slint:172` | Distinct hover role. |
| S9 | Bubble focus border uses the same accent ring as the composer. | `app/ui/message.slint:62` | Style as a selection highlight, not a focus ring. |
| S15 | Resizing from 740–1059 px to below 740 px with focus in the browser hides the shell, but the profile-sheet focus redirect does not fire, so focus can sit in a hidden control. Unverified: MCP cannot resize. `tools/check-ui-mcp.py --layers` still hard-codes the strip height (116) and rail+browser width (342). | `app/ui/app.slint` (`changed profile-sheet`), `tools/check-ui-mcp.py` | Redirect focus whenever covered content changes; read widths from the UI tree in the script. |
| S16 | Connection strip follow-ups: at 360×520 text 24 the status elides to "Sessio…" beside its actions; Retry and Sign in again have faint borders in dark mode; narrow Friends centres keyboard-selected rows on the full list height, ignoring the strip inset; the demo "List load failure" scenario inherits the previous scenario's connection state. | `app/ui/connection-notice.slint`, `app/ui/friends.slint:15`, `app/src/demo.rs` | Shorter visible status at narrow sizes; stronger action affordance in dark; use the visible height in Friends; reset demo state per scenario. |
| S18 | At 360×520 with 24 px text, friend rows overflow into each other (status text overlaps the next row). | `app/ui/friends.slint` | Row height from text metrics. |
| S19 | At 360×520 the content-sized Settings dialog leaves the composer visible beneath it; reads as a broken sheet. | `app/ui/layers.slint` | Full-window sheet below 740 px, content-sized above. |
| S20 | Shift+Tab from Navigation hits an unlabelled focus stop before Retry connection (pre-existing on `ca2ce40`). | `app/ui/chats.slint`, `app/ui/rail.slint` | Find the stop; label or skip it; extend the MCP reverse-Tab assertion. |

## M — Messages

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| M9 | Unread badge on the selected chat row is accent on selection (blue on blue). | `app/ui/chats.slint:98` | Invert on selected rows. |
| M11 | After clicking Latest, the next reply jump does nothing (no scroll, no Return pill); pre-existing on `1d12fba`. | `app/src/paging.rs` (`reset_latest`, `jump_to_message`) | Reset the jump state on Latest so the next jump works; MCP case Latest → reply jump → Return. |

## C — Composer, input, feedback

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |

## V — Verification debt

| ID | Problem | Fix |
| --- | --- | --- |
| V5 | No physical verification at all (Fcitx, scrollbar drag, 100/150/200 % scale, dark theme, notifications, tray, single-instance raise) and no account verification. | A person at the desktop, then assign the device slot. |
| V6 | `ux-acceptance.md` rows marked `yes` over the problems above: VIS3, FB1, FB3, WIN4, IN5, MSG5, RD3 (live), CMP7 (M), OS9, OUT2 (fixed slot), SH8. | Reset those rows when their fix lands; do not pre-emptively edit the history. |

## Resolved

| ID | Resolution |
| --- | --- |
| T1 | RPCs go through one pooled `ureq` 3 agent per `Api` (rustls, HTTP/1.1, no async runtime) with the curl path's safety properties, prompt cancellation including DNS, and a single pre-body retry on stale reused connections. Live account unverified. |
| T7 | QR encoded in-process with `qrcodegen` (PNG for the app and `--qr-file`, black-on-white ANSI or plain text in the CLI); kitty output removed. Verified by independent zxing decode. |
| T8 | Already correct at `05cd26d`: `app/src/links.rs` spawns `xdg-open` on a helper thread and waits on it. The audit row was stale. |
| T10 | `line-auth` resolves defaults through the app's XDG `Locations` (moved to `src/locations.rs`), lazily, honouring the saved paths choice. |
| V1 | Demo scenarios moved into an F12 dialog (`app/ui/demo-scenarios.slint`); the demo shell renders as production, apart from the title hint. MCP checks pass, each flag on a fresh instance. |
| S1 | The profile is a panel that narrows only the conversation from 1060 px (rail + browser + 398 px conversation + 320 px panel, derived from shared widths); 740–1059 px it is a sheet over the conversation with the browser kept; below 740 px a full-window sheet. Always below the connection strip. |
| S2 | The profile closes on destination switch and on navigation to any other chat; it stays only for its own chat. |
| S3 | Open chat switches to Chats, selects and minimally reveals the row (clamped scroll), reuses existing rows, and shows "No messages yet" for a friend with no history. |
| C1 | Ctrl+Enter sends beside the non-modal panel; only modal layers, including the conversation sheet, block it, without inserting a newline. |
| T3 | The SSE stream runs on the sync thread through the shared ureq transport on a fresh, unpooled connection; lines are read as they arrive, stop is honoured within 50 ms, EOF/reset reconnect at once, and the 30 s ping watchdog counts only socket silence (not callback time, not buffered data, and partial-line trickles still expire). Any 200 opens the stream, as with curl. |
| T2 | Media is fetched by three named fetcher threads over a shared ureq agent with its own resolver, a 256-key deduplicated queue that serves the visible range and open chat first, and a separate asset-event channel so UI commands keep the worker's full command capacity. Decode/encode run outside the cache lock. No component spawns curl any more. Remaining: a failed read outside the lock may remove a file another fetcher just stored (later cache miss only). |
| T6 | Reconnects are a quiet inline "Connecting…" header state plus a transient "Reconnected" (no notification or sound); only Offline, Signed out and Session expired raise the strip. Startup failures carry a Session reason so Sign in again stays reachable, also after a failed Retry. |
| S7 | The strip is one bordered line anchored below the headers over scrollable content only; the scroll range gains a compensated top inset so nothing moves and everything can be revealed; read exposure uses the visible height. Keyboard: forward Tab and F6. |
| C2 | List failures stay in the list footer with a Retry that re-issues exactly the failed request; wording matches the button. |
| M1–M4, M6–M8 | URLs render once, inline and activatable, in a measured rich flow: line breaks are computed once over the whole body with UAX #14 plus URL delimiters inside links, characters break only inside a run wider than the bubble, the 2048-run cap falls back to the selectable body, and widths come from a public 64× Text measurement with a bounded LRU cache. A click only focuses; the quote block is the jump target; failed rows keep mark and time beside the bubble with compact actions; attachments read name, size, action; outgoing bubbles have their own palette role (selection contrast 5.8–8.6:1). First frame and text-size change are faster than before; resize is 2.5–3.5× slower but under 10 ms. Cross-run drag selection is a documented deviation. |
| M5 | Not reproduced on `ded2def` (headless, live, text-size switch); no fix claimed. |
| S4, S14 | The duplicate rail Account button is gone; wide rail and narrow bar have exactly three destinations. |
| S6 | Settings has a title row with a close icon, sections as a tab list, content-sized, with scoped focus descriptions. Colours copied from Fluent rather than palette roles (minor). |
| S8 | The history pane focus ring is removed; rings remain only on the composer and the selected bubble. |
| S10 | Composer shows a "Message" placeholder, is at least two lines tall, and the reason caption appears only with a reason. |
| S11 | Latest and Return are side-by-side floating pills over the history bottom edge; the history height no longer changes. Focus order Latest → Return → composer, reversed on Shift+Tab. |
| S12 | Friends headers use chevron icons (no glyphs in labels) and empty sections are omitted. |
| S13 | Friend avatars fall back to the first grapheme. |
| C5 | Search Clear is an icon inside the field, shown only with a query; Tab from Clear follows the same narrow-offline rule into the connection strip. `SearchField.enabled` is unused (minor). |
| T4 | Media failures keep a typed class end to end (timeout, network, 403, 404/unsupported host, 5xx, rejected path, too large, decode, cancelled) with a plain reason; retriable classes show a centred "Retry image"/"Retry file" that refetches past cache and stored failure with a priority boost, transient classes are re-eligible after 300 s and retried once on sync reconnect at normal rank, permanent classes never refetch. Failure records are capped at 256. Minor: a local queue refusal reads as "Couldn't load this image". |
| C3, C4 | Production strings rewritten per model §9.2; manual Refresh buttons and the refresh icon removed, no string mentions Refresh. |
| V2 | Demo sends go through the production composer, outbox and history code to a local demo worker that answers like the server (Sent, then Incoming); F12 "Fail next send" (uncertain) and "Reject next send" exercise Resend/Cancel and typed Retry. Demo-only; the real worker and Retry path are unchanged. |
| V3 | Demo chats open at latest like a real chat; reading, rich and photo scenarios deliberately open at the top. |
| V4 | Fixtures are synthetic but realistic (mixed Japanese/English, link, long message, emoji, two-line rows) and dated relative to now: Today spread over ~3 h before now (compressed after midnight, DST-gap safe), Yesterday, this week, last year. Snapshot scenes use a fixed clock (2026-10-18 14:00). |
| M10 | Return restores the anchor against the cover inset only and clears the Pane residual first, so it lands at the pre-jump screen position with nothing hidden above it whether the connection strip appears, disappears or stays between jump and Return; a second jump works. |
| S17 | While a pill shows, the pill band is excluded from read exposure and the history gains equal bottom padding, so every row (including short last rows) can be scrolled clear and is marked read at latest. Minor: with Latest showing, the first wheel notch at the bottom only collapses the band. |
| B1 | Build cost: a `fast` profile (incremental, app crates unoptimised, dependencies optimised) and the Slint-generated code moved into its own `line-desktop-ui` crate. Warm rebuilds: Rust-only edit 30 s → 5.5 s release, 3 s → ~1 s fast; `.slint` edit ~33 s release, ~8 s fast. No measured runtime change. `tools/run-ui-debug.py --profile`; analysis and remaining proposals in `docs/build-iteration.md`. |
