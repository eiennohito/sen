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

Order of work: D first: it is basic functionality, and M13 loses messages
today. Then T, because every "slow" item below it is partly T. Then P, then
the UI groups in any order. V closes the loop.

## D — Local data and persistence

Recorded 2026-10-18 from a source audit (nothing run against the account).
Decision from the user: local history is a durable store, not a cache, and
at-rest encryption of local data (history, drafts, images) is not a goal: anyone
who can read the data directory also holds the session. Only the session
credentials stay sealed (T11). The implemented ownership and storage model is described in
[core-model.md](core-model.md). D0–D8 are resolved below.

## T — Transport and worker

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| T9 | wasmi executes every message encrypt/decrypt and request HMAC. The native trial measured roughly 11,000× faster message crypto, but replacing the blob remains blocked on SKB token/private-key loading and opaque MID conversion. | `src/runtime.rs`, [crypto-port.md](crypto-port.md) | The trial was removed; the blob remains in use. The account net worker owns and directly calls one runtime for live decode, history and signing; sends and receives share cached keys/channels, and signing keys are cached per version/access token. |

## P — Protocol gaps

Known gap: files outside the extension whitelist are not zipped.

Decided 2026-10-18: the traces for T5, P1–P3, P5 and P6 come from a live Chrome
capture that the user drives, recorded through the approved 127.0.0.1:9222
debugger. Keep only request/response shapes (methods, paths, field names,
`contentMetadata` keys, hosts); no tokens, IDs, message contents or media in Git.

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| P5 | Phase 6 implements group info, members/invitees, mute via full-object `updateChat` and confirmed leave via `deleteSelfFromChat`; operations 121/122, 127/128 and 41 queue chat refreshes. **live unverified**. | `detail.slint`, engine, Backend, migration 002 | Invite, accept/reject invitations and remove member remain blocked: untraced. Group editing remains out of scope. |
| P6 | Phase 6 implements relation-aware profiles, Rename (`updateContactSetting`), Add friend (`addFriendByMid`), fraud notice and Notifications; contact operations 4/49 queue contact refreshes. **live unverified**. | `detail.slint`, engine, Backend | Block, Report and Unblock remain untraced. Favourite is read-only; optional cover skipped (new channel auth); shared media remains blocked pending traces. |

## S — Shell and layout

S23 and S24 are resolved below.

## U — Profiles, members and groups

U0–U6 are resolved in Phase 6 under the user-approved [§3.4 design](ux-model.md#34-profile-member-and-group-surfaces-u0-approved).
The prototype block was replaced with the shared detail stack; see the Resolved rows.
Remaining protocol work stays in P5/P6.

## M — Messages

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
M13 and M14 are resolved by the core model; see the resolution rows below.

## C — Composer, input, feedback

C6 and C7 are resolved below.

## V — Verification debt

| ID | Problem | Fix |
| --- | --- | --- |
| V5 | No physical verification at all (Fcitx, scrollbar drag, 100/150/200 % scale, dark theme, notifications, tray, single-instance raise) and no account verification. | A person at the desktop, then assign the device slot. |
| V6 | Acceptance rows reconciled on 2026-10-18 (core model phase 3): resolved defects point to their resolution; remaining defects are `part` with their IDs. | Keep Impl separate from verification; physical and account checks remain pending (V5). The separate composer checker still assumes pre-6a Sending/Not sent image labels and an Edit button; its recovery section needs updating independently of the passing Phase 6 primary groups. |

## Resolved

Phase 7 basic chat flows follow [the decided visual design](visual-design.md#flows-added-after-the-audit). All new RPCs run through engine intents, the network worker and Backend; LineBackend remains **live unverified**.

| ID | Resolution |
| --- | --- |
| P3 | Phase 8: portal multiple-file chooser, clipboard images/file URIs and batched winit drops open the decided attachment dialog without sending. Original-quality photos (10 MB, over 1e8 pixels treated as files) and files (300 MB fallback; configurations are not fetched) retain the source path and worker-generated thumbnail. Ordered pending rows use the network worker, persisted request sequences, op 25/43 dedup, local progress deltas, Retry/re-upload, Discard and File missing. Media encrypts with the existing HKDF/AES-CTR/HMAC envelope; keyMaterial and fileName are sealed, with traced original/preview uploads and MEDIA_CONTENT_INFO/MEDIA_THUMB_INFO. FakeBackend supports delayed/failing uploads; one core flow covers two attachments and retry. **live unverified**; physical portal/clipboard/drop verification remains pending. |
| P2 | Phase 7: composer Sticker popover, cached owned packages with background refresh, package tabs and four-column image grid through the media cache. Sticker activation sends plain contentType 7 with STKPKGID/STKID/STKVER and no E2EE chunks, using the text-send Pending/Rejected/Uncertain and Retry/Discard lifecycle. Two procedural, original demo packages; core flow covers echo dedup and retry. **live unverified**. |
| New chat | Phase 7: Chats title action opens searchable friends, removable selected chips and Chat/Create group. One friend opens locally, with the first send creating the conversation; groups call createChat type 0 without self, show progress/failure and open the returned chat before details refresh. Core flow covers first send and group success/failure. **live unverified**. |
| U0 | Phase 6: user-approved §3.4 is the normative design, including relations, stack, focus, responsive placement and traced action states. |
| U1 | Phase 6: relation-aware Chat/Rename/Add friend, read-only Favourite, fraud notice, More → Copy name; own and unknown profiles are read-only. Writes use engine intents and Backend. **live unverified**; Block and Report remain untraced (P6). |
| U2 | Phase 6: header avatar/title, group sender avatar/name, member and Friends rows, Settings account and rail account open the shared detail stack; group Friends rows select that chat. |
| U3 | Phase 6: prototype block replaced by detail.slint: accent cover tint, circular 96 px avatar/viewer, rich name/status, original name, compact icon actions and header. Optional cover fetch skipped because it needs channel auth. |
| U4 | Phase 6: group info → Members, Notifications switch and confirmed Leave; full getChats wire object is stored for updateChat. Rooms omit writes. Left chats remain read-only in the list. **live unverified**. |
| U5 | Phase 6: searchable avatar/name/status rows, Friend marker, self first, friend Chat on hover/focus; invitees form Invited (N), without profile actions. |
| U6 | Phase 6: controller-owned DetailPage stack (Profile, GroupInfo, Members), one pop/close rule, one opener-focus restoration adapter; same stack at 1060/740 breakpoints. §8.2 updated. |
| T5 | Phase 7 adds send/cancel from a six-choice popover and React menu, own-choice highlighting, optimistic actor replacement and failure rollback with one transient danger line. Operation 139 remains actor-idempotent. **live unverified**. Phase 5: traced 139 replaces/removes the own actor entry through `set_reactions`; 140 uses an explicit actor MID when present, otherwise queues a quiet Recent refresh. Raw actor entries persist alongside display grouping; no notification, unread increment or view jump. FakeBackend raw injection and core-flow regression cover replacement, removal and actorless refresh. **live unverified** (140 shape remains inferred). |
| P1 | Phase 5: OA images without SID/OID use PREVIEW_URL for thumbnails and DOWNLOAD_URL for originals, restricted to exactly `https://manager.line-scdn.net/`, unencrypted and without LINE auth headers. Existing asset URL tests cover host rejection and local transport headers; demo Protocol fixtures exercises thumbnail/original through FakeBackend. Generic image 404 wording replaces the OA sender special case. **live unverified**. |
| P7 | Phase 5: own confirmed, non-withdrawn messages offer Unsend in their context menu. A modal asks “Unsend message?” / “It will be removed for everyone.” with Unsend and Cancel; Escape cancels. The worker calls traced `unsendMessage [reqSeq, serverId]`, withdraws only on success, and ignores the later 64 echo. Failures keep the row with one transient “Couldn't unsend.” beside it. Withdrawn rows read “Message unsent”. FakeBackend, core flows and the layers MCP group cover success/failure and the dialog. **live unverified**. |
| D0 | Phases 1–2: SQLite is the sole message owner; controller projections and keyed presenter consume engine deltas. |
| D1 | Phase 1: durable per-account SQLite history, unbounded messages. |
| D2 | Phases 1–2: pages, live operations, echoes and send results merge into the store before deltas. |
| D3 | Phases 1–2: account identity, chats, contacts, members, receipts and sync revision persist in SQLite. |
| D4 | Phase 2: local open precedes connection; validated identity selects the stored account offline. |
| D5 | Phase 2: applied revisions persist transactionally and seed reconnect; replay suppresses duplicate attention. |
| D6 | Resolved by removal in phase 2: no login-key-derived local state or message-page encryption remains. |
| D7 | Phase 2 removed LocalStateStore, MessageCache and encrypted pages; drafts, outbox and preferences use the MID-scoped store. Phase 3 removes unused paths. No legacy import or deletion. |
| D8 | Phase 3: media is a 128 MiB / 4096-file LRU; disk hits refresh mtime, pruning evicts oldest mtime, no age expiry. Decoded cache: 24 MiB. |
| M13 | Phases 1–2: stable message keys and monotonic store merges retain sends across chat switches and stale pages; engine/controller regression coverage. |
| M14 | Phases 1–2: closed-chat arrivals persist; opening reads the same rows and schedules refresh when the store is behind; engine/controller regression coverage. |
| T1 | RPCs go through one pooled `ureq` 3 agent per `Api` (rustls, HTTP/1.1, no async runtime) with the curl path's safety properties, prompt cancellation including DNS, and a single pre-body retry on stale reused connections. Live account unverified. |
| T7 | QR encoded in-process with `qrcodegen` (PNG for the app and `--qr-file`, black-on-white ANSI or plain text in the CLI); kitty output removed. Verified by independent zxing decode. |
| T8 | Already correct at `05cd26d`: `app/src/links.rs` spawns `xdg-open` on a helper thread and waits on it. The audit row was stale. |
| T11 | Desktop Secret Service seals session files with AES-256-GCM (format `line-desktop-sealed-session`, mode 0600). Its 32-byte key is one item in the default collection (`application=line-desktop`, `purpose=session-key`, `key-id`), reused by later saves. Keyring mode refuses plaintext with a typed saved-session failure. `LINE_DESKTOP_CREDENTIALS=plaintext-file` is the explicit development store and refuses sealed files; `tools/run-ui-debug.py` selects it for its private bus. Credential tests use in-memory keys and a synthetic Secret Service, never the real keyring. |
| T10 | The app uses fixed XDG `Locations` in `src/locations.rs`; `--state` and `--wasm` override individual paths. |
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
| P5a | Read-only members: groups and rooms have a Members target on the header title ("Members, N", one count subtitle), opening a panel/sheet with the profile panel's placement rules; members listed self first then by name, searchable, avatars, "Unknown member" when unresolved; member → profile → Back (Escape steps back, then closes); Open chat only for friends or existing chats. Names come from the existing `getChats`/`getContactsV2` shapes in a separate bounded cache; failed lookups are not cached and Retry repeats them. |
| M11 | Latest keeps the measured row geometry (rows whose layout didn't change never re-report it), so the next reply jump scrolls and shows Return; obsolete jump highlight and errors are cleared. |
| S15 | Focus moves to visible content whenever a layer starts covering it (auth, viewer, settings, profile, members), in both width directions; no change when nothing is covered. `check-ui-mcp.py --layers` reads the geometry from the UI tree. |
| S20 | The connection-strip and shell wrappers no longer take Tab focus; Send and Back have focus descriptions; no unlabelled stop remains in the forward or reverse chains. |
| S5, S9, M9 | Chat rows have a hover role distinct from selection (3.1–3.2:1 against the pane); the selected bubble is a tinted fill with a 2 px selection border instead of the composer ring; the selected row's unread badge inverts. Minor: hover is heavy (dark text 12:1 → 4.5:1) and a selected outgoing bubble loses its green tint. |
| S16, S18, S19 | Narrow strip shows a compact status ("Expired") with the full status in the accessible label; Retry/Sign in again borders are visible in dark (light border and pressed style also changed); narrow Friends centres keyboard selection on the visible height; demo connection scenarios reset prior failures; Friends row height comes from text metrics; Settings is a full-window sheet below 740 px. Minor: compact status says "Sign in" when signed out (reads as an action, duplicates the button) and switches on width only, not text size. |
| M12 | Chat-list previews render LINE emoji and media like the history: one shared conversion feeds the initial list, refreshes and sync updates; emoji images inline with history's reserved square and fallback; small rounded thumbnails for photos/albums/videos (cover) and stickers (contain) with a short caption, text-only rows keep their glyph; accessible label, search and notifications use the placeholder text (`[笑]`), stickers notify with STKTXT. Only visible rows without a picture are requested; one downscaled image per key, visible rows kept before a ±2-row margin, budget from viewport geometry capped at 16 MiB. Minor: a request lost or failed while its row stays visible shows the text fallback until the row is revisited or the fetcher's reconnect retry; `scrolling_previews_request_only_missing_pictures` doesn't exercise the missing-only filter (add a back-and-forth step). |
| S21, S22 | Every surface (Chats, Friends, conversations, members/profile panels and sheets, Settings; narrow and wide; with and without the strip) has one Tab cycle with Shift+Tab its exact mirror; the reading order is recorded in ux-model §8.2. Narrowing moves focus to the draft only when it was in the rail or browser that becomes hidden. The compact strip status reads "Signed out" and is chosen by whether the full status fits at the current text size. `check-ui-mcp.py --focus` asserts the exact cycle and mirror from every stop, resets its state, and no longer exits early. |
| C6 | Composer and search fields share a dimmer placeholder colour, hide it on focus, and retain accessible placeholder text. |
| C7 | Secondary text uses explicit scheme colours (#626262 light, #b0b0b0 dark), each exceeding 4.5:1 on the background; primary content keeps foreground. |
| S23 | Chats filters are one Tab stop with Left/Right and Enter/Space; Friends category headers participate in list arrow navigation and toggle with Enter/Space; §8.2 and the focus checker cover both. |
| S24 | Focused controls use enabled/visible focus descriptions and Send returns focus before submitting; the focus checker no longer polls around stale descriptions. |
| P4 | Links detect bare domains with simple TLD rules, case-insensitive schemes and rendered text across inline emoji runs, excluding trailing punctuation and unmatched parentheses; existing link tests cover these cases. |
