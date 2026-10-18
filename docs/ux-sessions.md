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
| T5 | Reaction counts never update live: the sync decoder handles no reaction operation; counts change only when a message page is refetched. | `src/client/desktop/sync.rs` (no match for reaction) | Decode the reaction operation into an in-place row update; it is a quiet event in the attention matrix. Traced 2026-10-18 ([protocol-traces.md](protocol-traces.md#reactions-t5)): operation 139 SEND_REACTION (own, any device; `param1` message id, `param2` JSON `{chatMid, curr}`), 140 NOTIFIED_SEND_REACTION for others (same shape inferred, not observed); sending is `react`. |
| T9 | Crypto runs the Chrome WASM blob in the `wasmi` interpreter for every encrypt and decrypt; the sync thread instantiates a second runtime; the user must supply `ltsm.wasm`. | `src/runtime.rs:506` | Trial done: `src/native_crypto.rs` ports derivation, HMAC, AES-GCM, Letter Sealing V1/V2 and X25519, proven byte-for-byte against the blob (see [crypto-port.md](crypto-port.md)); about 11,000× faster on a V2 decrypt. Blocked: `SecureKey` (SKB) keys are opaque, so stored-key loading, token recovery and MID decoding still need the blob. Next: decide whether to switch message crypto to native while keeping wasmi for key unwrapping. |

## P — Protocol gaps

Decided 2026-10-18: the traces for T5, P1–P3, P5 and P6 come from a live Chrome
capture that the user drives, recorded through the approved 127.0.0.1:9222
debugger. Keep only request/response shapes (methods, paths, field names,
`contentMetadata` keys, hosts); no tokens, IDs, message contents or media in Git.

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| P1 | Official-account media uses a different route. The client only builds `obs.line-apps.com/r/talk/{SID}/{OID}/preview` with defaults `m` and the message id, so OA images 404 and show as "Preview unavailable". The map records that `DOWNLOAD_URL` variants are refused on purpose. No OA message exists in `dumps/`. | `src/client/desktop/assets.rs:106`, `docs/chromium-ui-map.md:351` | Trace one OA image message (its `contentMetadata` keys and resolved host), add that host to the allowlist, build the right URL. Interim wording "Not available for this sender" (any image 404) landed with T4. Traced 2026-10-18: OA images have no `SID`/`OID`; `contentMetadata.PREVIEW_URL`/`DOWNLOAD_URL` point at `manager.line-scdn.net` and Chrome fetches `PREVIEW_URL` (`…/clearpreview`) directly. Allow that host, use the metadata URLs; revise the "DOWNLOAD_URL refused" rule in `chromium-ui-map.md:383` to this host only. |
| P2 | Sticker sending does not exist: no picker, no send path. Listed as "later". For a LINE client it is a daily-use blocker. | `app/ui/composer.slint`, `app/src/composer.rs` | Trace one sticker send (STKID/STKPKGID/STKVER shape is in the HAR), implement the send, then a picker as a panel in wide and a sheet in narrow. Traced 2026-10-18: send is plain `sendMessage` contentType 7 with `STKPKGID/STKID/STKVER/STKTXT`, no E2EE; owned packages from `getOwnedProductSummaries`, package contents from `productInfo.meta`. |
| P3 | Attachment sending, paste and drop of media do not exist (CMP8 media, CMP9, OS10). | composer | After P2. Traced 2026-10-18: `acquireEncryptedAccessToken`, `negotiateE2EEPublicKey`, `determineMediaMessageFlow`, encrypted upload to `obs…/r/talk/emi/<oid>` (image + preview) or `/emf/<oid>` (file) with `x-obs-params`, then `sendMessage` (contentType 1 or 14, `SID`/`OID`, key material in E2EE chunks). The encrypted file format and key-material JSON still need checking against the WASM. Chrome zips files with extensions outside the server's `function.file.extension.whitelist` after a "Send compressed" prompt; decide whether to mirror that. |
| P5 | Group membership changes are not possible: no pending invitations to accept or reject, no invite, remove, leave, or group name/image edit; membership freshness follows chat refreshes only (no local trace of membership operations, roles or join times). Minor from P5a: the `members-offline` snapshot scene at 980×720 renders wrong (no strip, stray Retry, sheet at y≈198; live is correct), and the conversation header above the strip shows an empty band behind the members sheet. | `src/client/desktop.rs` (`members`), `docs/chromium-ui-map.md:235–270`, `app/src/main.rs` snapshot scenes | Invitations, then invite/remove/leave/rename, each needing a trace and confirmation design; mutating calls cannot be verified without the device slot. Fix the snapshot scene to match live. Traced 2026-10-18: `updateChat` (full chat object; operation 121), `deleteSelfFromChat` (127, 41); invite/remove/accept/reject named in `main.js`, shapes not captured. |
| P6 | Profile actions have no protocol path (U1, U4): 1:1 chat creation with a non-friend (`createChat` appears in Chrome `main.js`), Add friend (`findAndAddContactsByMid`/`addFriendByMid`), Block/Unblock (`blockContact`/`unblockContact`), favourite/rename (`updateContactSetting`), Mute chat, cover image field, shared photos/links/files ("Show all images", "View all URLs"). Names from the installed Chrome source; request shapes unknown. | `docs/chromium-ui-map.md:72-73,219,233` | Add to the live Chrome capture list; mutating calls verified only with the device slot. Traced 2026-10-18: there is no non-friend chat in LINE (profile offers Report, Block, Add friend + fraud notice from `getTargetProfileNotice`); `addFriendByMid`, rename via `updateContactSetting(…, 2, name)`, cover via `timeline/getCover` + `obs…/r/myhome/c/…`; Block/Unblock not captured. |
| P7 | Unsending own messages is not possible (receiving an unsend works: "Message withdrawn" row). | `app/src/links.rs:110-142`, composer/message actions | Traced 2026-10-18: `unsendMessage [reqSeq, messageId]`, operation 64 (own) / 65 (others). Add an Unsend action on own messages with confirmation; render unsent messages as LINE does, a centred system pill "Message unsent." rather than a bubble. |

## S — Shell and layout

S23 and S24 are resolved below.

## U — Profiles, members and groups

Recorded 2026-10-18. The user's verdict: the profile surfaces look bad and have
no flows (e.g. no way to message a group member). Audit screenshots (local,
not in Git): `/tmp/claude-1000/prof/`. The user's verdict on the profile page
specifically: it looks bad; it is the initial PoC design and was never
designed. Replace it, do not polish it: no incremental fixes to
`layers.slint:110-125` count as progress on U.

ux-model.md only specifies where profiles sit (panel/sheet placement,
§85–116), never what they contain or how they look, which is how the PoC
design survived. U0 comes first.

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| U0 | No design exists for profile, member and group surfaces: content, hierarchy, sizes, actions and states are unspecified. | `docs/ux-model.md` §85–116 | Write the design into ux-model before any code: reference the real LINE client (screenshots of its profile, group info and member list, taken during the live Chrome capture, kept locally), list content per surface (own, friend, non-friend member, unknown member, group), action row per relation, empty/unknown/loading states, wide and narrow mock-ups. Reference screenshots taken 2026-10-18 (friend, non-friend, group info, invite picker): `artifacts/captures/2026-10-18T07-17-15-989Z/*popup*.png`, summarised in [protocol-traces.md](protocol-traces.md#reference-screenshots-u0). The user reviews it before U1–U6 are implemented. |
| U1 | Dead ends: a member's profile has no action unless they are already a friend or already have a chat (`app/src/members.rs:82-85`, `app/ui/layers.slint:122`); no Add friend, no message, no More. Every profile's only possible action is Open chat. | as cited | Per relation, as LINE does (reference screenshots, P6 trace): friend — Chat, favourite, rename; non-friend — Add friend, Block, Report, fraud notice, no chat; More for Copy name. |
| U2 | Missing entry points: 1:1 header title is plain text (`app/ui/conversation.slint:55`); sender avatars and names in a group are inert (`app/ui/message.slint:66-83`); no own profile (Settings shows only the account name, `layers.slint:216`) though `getProfile` is already fetched. | as cited | Header title, sender avatar/name and member rows all open the same card; own profile (read-only) from Settings or the rail. |
| U3 | Layout defects, seen in screenshots: hidden Back and avatar still reserve space (~44 px band, ~80 px hole; `layers.slint:114,116`); the unknown-member fallback avatar is a full-width blue bar with "U" (`:118`); the status message floats mid-panel inside a stretching ScrollView (`:120`); caption "Friend/Member profile" sits between avatar and name (`:117`); Close/Back are full-width text buttons; Open chat is a pale button pinned to the window bottom; avatars pixelated at 80 px; narrow sheets mostly empty; emoji in names/status dropped. | `app/ui/layers.slint:110-125` | Rebuild from the U0 design; the audit's starting proposal, to be checked against the LINE reference: cover band, centred 96 px circular avatar (opens in the viewer), name, "name set by friend" when renamed, status message, icon action row; absent parts collapse; header with back arrow and × icon. Render names/status through the rich-text path so emoji appear. |
| U4 | Groups have no info surface: Members is reached only through the header title; the "Group profile" from Friends shows name and status only, no member count or path to members; a group profile can open beside an unrelated conversation. | `app/ui/members.slint`, `app/ui/layers.slint`, `app/src/browser.rs` | Group info panel: photo, name, member count, Members row (drill-in), then Photos/Links/Files and Mute (P6), and a danger-styled Leave with confirmation (P5). Header icons for members and More. Opening a group's info from Friends opens that group's chat with it. |
| U5 | Member list rows show only a name: no status line, no Friend marker, no row action. | `app/ui/members.slint` | Avatar, name, status line, Friend badge; hover/focus Chat action. |
| U6 | Profile, members and group profile share one `profile-open` flag, a panel at ≥1060 px and a sheet below, with focus special-casing across `app/ui/app.slint:212-262`, `app/ui/state.slint:33-35`, `app/ui/members.slint:15-24`. | as cited | One right-panel navigation stack (group info → members → member profile), one Back/Escape rule, one focus-restoration path; same stack as a sheet in narrow layouts. Update ux-model §8.2 Tab cycles. |

Buildable now from existing data: U2, U3, U5, U6, and U1/U4 without the traced
actions. Needs P6 first: messaging a non-friend, Add friend, Block, Mute,
shared media.

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
| V6 | Acceptance rows reconciled on 2026-10-18 (core model phase 3): resolved defects point to their resolution; remaining defects are `part` with their IDs. | Keep Impl separate from verification; physical and account checks remain pending (V5). |

## Resolved

| ID | Resolution |
| --- | --- |
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
| T11 | Decided 2026-10-18: desktop Secret Service, with a development store for agents. `src/credentials.rs` seals the session file with AES-256-GCM (format `line-desktop-sealed-session`, mode 0600); its 32-byte key is one item in the default collection (`application=line-desktop`, `purpose=session-key`, `key-id`), reused by later saves so `before-sign-in` backups stay readable. Every session read/write goes through it. `Client::open` and `line-auth status` seal a legacy plaintext file only after the new key reads back and the envelope round-trips, and never over a concurrent save; plain loads never write. Sealed files are never downgraded; a refresh on an unsealed file without a reachable keyring stays plaintext rather than being lost; a new sign-in checks the keyring before asking the phone. `LINE_DESKTOP_CREDENTIALS=plaintext-file` keeps the old mode-0600 file per `--state` path for agent and debug instances and refuses sealed files; `tools/run-ui-debug.py` sets it in its private bus. Unit tests plus a private-bus contract against a synthetic locked keyring (unlock prompts, item attributes) pass. Not run against the real keyring or the account's `session.json`, which migrates at the first authorised start. Startup failure from a locked/absent keyring still shows the generic session failure. |
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
