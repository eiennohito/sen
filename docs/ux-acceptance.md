# Native client UX acceptance

Status as of 2026-10-18, implementation commit "core model phase 3".
Requirements derive from [ux-model.md](ux-model.md); section numbers refer to
it. Open problems and their fix order live in [ux-sessions.md](ux-sessions.md);
V6 rows now point to resolved defects or carry `part` with the remaining defect IDs.

## How to read and update this file

Each requirement has two columns:

- **Impl**: `yes` the behaviour exists in code; `part` some of it exists;
  `no` it does not. Set by the implementing session with a commit reference.
- **Verified**: how the behaviour was exercised. `S` synthetic controller test
  (`cargo test`), `M` offline Slint MCP check (`tools/check-ui-mcp.py`),
  `P` physical desktop input (pointer, scrollbar, Fcitx, scale), `A` real
  account, `—` not verified. A requirement is accepted only when every
  listed verification kind that applies has passed and is logged below.

Implemented is not verified. A session that cannot run a verification kind
writes `—` and says why in its hand-off. Account-dependent checks stay
blocked while Chrome holds the device slot (see Blockers).

Current totals: 120 requirements. Impl `yes` 92, `part` 26, `no` 2.
Current changes have no physical or account verification; historical account
observations below do not verify the current build.

## Blockers

- **Device slot.** LINE allows one Chrome-type secondary device. Chrome holds
  it; native pairing would sign Chrome out. All `A` verification waits for
  the user to assign the slot to the native app.
- **Live compatibility unverified.** Source-backed SSE, token refresh, unread/read
  and older-history adapters are implemented with synthetic coverage. Their
  behavior against the account remains unverified while Chrome owns the slot.
  Native mute has no known server source; unknown is not reported as a saved
  server preference. See the P1–P4 handoffs and model §7.1.
- **Physical verification** needs a person at the desktop: Fcitx, scrollbar
  dragging, 100/150/200 % scale, dark theme, notifications.

## SH — Shell and layout (model §3, §10)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| SH1 | Rail with icon destinations; current destination shown as selected, not disabled; Settings and account at the rail bottom | yes | S M |
| SH2 | Browser pane, conversation pane, and detail panel scroll independently; wheel input never spills across panes | yes (independent browser/history/profile surfaces) | M (base panes and expanded S7 private virtual-KWin --layers) |
| SH3 | Header and composer stay visible while history scrolls | yes | M |
| SH4 | One scrollbar style across panes; thumb ≥ 10 px; appears when content overflows; no text rewrap when it appears | part (shared Pane for chats, friends, history and native composer; standard Settings/profile/sign-in scrollbars remain) | S M |
| SH5 | Window and pane resizing keep selected chat, draft, focus, and reading anchor | part | S |
| SH6 | Narrow layout below 740 px: Back in header returns to the browser at its previous offset; unread reachable from the conversation pane | yes (Back icon, guarded Alt+Left and mouse Back; S8 unread badge) | S M |
| SH7 | Minimum 360×520 at the largest text size has no overlapping controls | part (verified at 24 px only by snapshot) | M |
| SH8 | Settings is a wide dialog; detail stack is a panel ≥1060, conversation sheet 740–1059, full sheet below 740 | yes (Phase 6, U6) | M (offline private --layers, default + --wide); live unverified |
| SH9 | Window title is "LINE" (optionally with unread count), never the account name | yes | S M |

## NAV — Conversation list, friends, search (model §3, §4.2, §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| NAV1 | Switching destinations keeps the open conversation and draft | yes | S M |
| NAV2 | Each destination keeps query, filters, expansion, focused result, and offset for the session | yes | S M |
| NAV3 | Chat row: avatar, name, time, one preview line, unread badge, mute and failed-send markers, draft marker; metadata never covers name or preview | yes (mute marker only when known) | S |
| NAV4 | Outgoing preview is prefixed; previews are single-line; times follow model §9.2 | yes | S M |
| NAV5 | Category filter All / Friends / Groups / Official (as the protocol allows) without closing the open chat | yes (All/Friends/Groups; Official awaits protocol distinction) | S M |
| NAV6 | Arrivals reorder rows without moving one under an active press or menu; the visible anchor is preserved; selection kept by identity | yes | S |
| NAV7 | Opening a chat keeps unread information visible until content is exposed | yes | S |
| NAV8 | Loading more conversations keeps the list offset; loading, end, failure, retry are distinct; retry never duplicates rows | part (no end state) | S |
| NAV9 | Refresh keeps cached rows and selection; progress and errors are local to the list footer | yes (cached rows retained; local footer progress/error and Retry) | S |
| NAV10 | Search scope is stated in the placeholder or empty state, not as a permanent caption | yes | S M |
| NAV11 | Up/Down move through results, Enter opens, Escape clears, all gated on IME composition; clearing restores the prior offset | yes | M |
| NAV12 | Filtering may hide the open chat without closing it or touching the draft | yes | S M |
| NAV13 | Friends list uses list rows with section headers, not buttons; a click opens a profile panel; nothing sends | yes (list rows and collapsible sections; profile panel in S7) | S M |
| NAV14 | Friends load and refresh failures keep loaded rows and show a local retry | yes | S |
| NAV15 | Alt+Up/Alt+Down switch conversation; Ctrl+F focuses search; Ctrl+1/2/3 switch destinations | yes (Ctrl+1 Friends, Ctrl+2 Chats; unavailable Ctrl+3 omitted per model §12) | S M |

## RD — Reading and history (model §5)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| RD1 | First open of a read chat shows the bottom; first open of an unread chat shows the unread divider with context | yes (bounded unread-boundary discovery; unknown stays explicit) | S |
| RD2 | Reopening within the session restores the previous anchor and at-latest state | yes | S M |
| RD3 | Incoming messages move the view only while at latest; receipts, reactions, media, and previews never move it | yes ([T5 resolved](ux-sessions.md#resolved): 139 in-place; actorless 140 queues Recent) | S M (offline; live unverified) |
| RD4 | Away from latest, a Latest control shows the count of incoming messages appended below the viewport; older loaded messages do not count | yes | M |
| RD5 | Latest and Send go to the bottom and resume following; typing, confirmations, and receipts do not | yes | M |
| RD6 | Older history loads before the top is reached, one request per boundary, stops at the beginning | yes (S10) | S M |
| RD7 | Loading, failure with Retry, and beginning-of-history are items inside the history that do not shift content | part (S5 recent loading/error overlays a fixed 24 px strip; older-history items S10) | S M |
| RD8 | Media load, failed media, font change, reaction change, and withdrawal keep the anchor; a vanished anchor falls back to its neighbour | yes (S10 layout anchor and neighbour fallback; phase 5 reactions/Unsend) | S M (offline); live unverified, physical V pending |
| RD9 | Switching chats during a load never flashes the previous chat or moves the new chat | yes | S |
| RD10 | Dragging a scrollbar or selecting text suspends following; release resumes only if at latest | part | M (simulated) P— |
| RD11 | Date dividers (Today, Yesterday, weekday, date) and five-minute sender groups per model §9.2 | yes | S M |
| RD12 | Bursts do not bounce the viewport; any animation is interruptible and honours reduced motion | yes (no animation) | M |
| RD13 | History is one Tab stop; arrows, Home, End, PageUp, PageDown scroll it; messages are click-focusable only | yes (S5 explicit surface traversal on Slint 1.18; focus ring and history keys) | S M |
| RD14 | The loaded window is virtualised or windowed before exceeding a few hundred rows | yes (S10: 200 UI rows, 600 retained per view, eight views) | S M |

## MSG — Message content and actions (model §6, §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| MSG1 | Pointer and keyboard selection inside a message, Copy without entering an editor | part (plain messages selectable; inline-image messages use Select text to expose the intact fallback text for selection; cross-run drag selection unavailable in Slint 1.18) | S M (selection mode for rich messages) |
| MSG2 | Copy yields visible text with line breaks and emoji fallback, no timestamps or labels | yes (rich Copy message and Select text retain the original fallback text) | S M |
| MSG3 | Right click keeps an existing selection and targets the message; incoming updates keep selection and menu | part (stable message identity preserves unchanged selection/menu; native menu has no open-state callback for full mutation deferral) | S M (unchanged-row and selected-body chat-switch scenarios) |
| MSG4 | Context menu offers Copy selection, Copy message, Copy link, Open photo and confirmed-own Unsend and React as applicable; menus stay on screen | part (S5 URL actions add full address, Open link and Copy link; P7 Unsend confirmation and Phase 7 React implemented, live unverified) | M (layers covers Unsend menu, cancellation, success and failure) |
| MSG5 | Links are detected, shown distinctly, inspectable (full URL), and opened only on deliberate activation | yes (synthetic links tests and reading checker) | S M |
| MSG6 | Reply previews jump to the referenced message with highlight and a return path | yes (local targets jump immediately; absent originals load by server ID) | S M (retained replies) |
| MSG7 | Photo, video, voice, file, location, contact, sticker, system, withdrawn, undecryptable, and unknown content each have a distinct placeholder or presentation; one bad message hides nothing | yes (typed rows; file name/size/Save, video/voice duration, contact name, literal system text, withdrawal “Message unsent”; no playback) | S M |
| MSG8 | Inline LINE emoji render as runs within text | part (native measured grapheme/image flow; cross-run ligatures/contextual shaping unavailable; static bitmap fallback for animated emoji; Select text switches to the intact fallback body for selection; bodies over 2048 runs use plain fallback) | S M (partial implementation as listed) |
| MSG9 | Accessible label of a message includes direction, sender, time, and delivery state | yes (S5 incoming sender with conversation fallback; outgoing time and delivery) | S M |
| MSG10 | Stickers have no bubble; photos keep aspect ratio in a reserved frame; reactions sit under the bubble | yes (Phase 7 reaction pill, three icons and total) | M |
| MSG11 | React from hover/focus or context menu; six choices, own highlight and repeat-to-cancel; optimistic update and transient failure rollback; op 139 dedup | yes; live unverified | S; compose MCP |

## CMP — Composer (model §8.1, app-design IME checks)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| CMP1 | Enter inserts a newline by default; Ctrl+Enter sends; Shift+Enter always newline; "Enter sends" is a preference | yes | S M |
| CMP2 | No shortcut fires during IME composition; Fcitx preedit, candidates, cancel, and refocus pass the app-design table | part (composition guards implemented; physical Fcitx table pending) | S P— |
| CMP3 | Composer grows to one third of the pane then scrolls; caret and last line visible; Send stays reachable | yes | M |
| CMP4 | Send is disabled for whitespace or no chat without instructional captions; offline, storage failure and over-size show actionable feedback | yes | S |
| CMP5 | Switching chats keeps text, caret, selection, and scroll per chat; clicking the open chat changes nothing | yes (text, byte-offset caret/selection, scroll; unchanged-chat guard) | S M |
| CMP6 | Drafts survive restart in private storage; nothing moves between chats or accounts | yes | S |
| CMP7 | After submit the composer is empty, focused, and a late result cannot alter newer text | yes (synthetic submit and focus checks) | S M |
| CMP8 | Pasting text keeps line breaks; pasting or dropping media opens the attachment preview | yes; media live unverified | S M (injected local paths); physical clipboard/drop P— |
| CMP9 | Attachment selection shows a preview with destination, remove, cancel, Send; nothing transmits on selection or drop | yes; live unverified | S; compose MCP (preview/remove/send/progress), light/dark screenshots |
| CMP10 | Shortcut help is in Settings, not a permanent caption under the composer | yes | S M |
| CMP11 | Sticker popover uses cached owned packages, tabs and four-column images; activation sends immediately and closes; Escape closes | yes; live unverified | S; compose MCP |
| CMP12 | New chat selects one friend locally; first send works without an existing chat. Multiple friends create a named group with progress/failure and open on success | yes; live unverified | S; compose MCP |

## OUT — Outgoing lifecycle (model §6)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| OUT1 | Submit creates a durable Pending item before any network call | yes (text and stickers) | S |
| OUT2 | Pending, Confirmed, Read, Rejected, Uncertain are visually and accessibly distinct in a fixed slot | yes ([M6–M8 resolved](ux-sessions.md#resolved): fixed delivery slot) | S M (Sending, Sent, Not sent, Unconfirmed) |
| OUT3 | Confirmation updates the same item; refresh, reconnect, restart never duplicate it; Confirmed never implies Read | yes (Phase 7 adds sticker echo/retry coverage) | S |
| OUT4 | The user can keep composing while sends are pending | yes | S |
| OUT5 | Definite rejection is classified separately from an uncertain result | yes | S M |
| OUT6 | Rejected offers safe Retry and Edit; Uncertain offers Resend with duplicate warning and Edit; Edit never overwrites a newer draft | yes (safe Retry; duplicate-confirmed Resend keeps original; Edit confirms Replace/Append) | S M |
| OUT7 | A send failure does not change the app's connection state or show the connection banner | yes | S M |
| OUT8 | Restart turns Pending into Uncertain, never into a draft | yes | S |
| OUT9 | Receipts update quietly; group reader counts only with protocol support | yes (quiet exact per-reader ranges, distinct-reader counts only when known) | S; live receipt A— |

## UNR — Unread and read (model §7.1–7.3)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| UNR1 | Unread counts and mute flags come from the server list and live events; unknown shows as a dot | part (server unread; Chrome mute is local storage, native server mute unknown) | S |
| UNR2 | Read eligibility follows model §7.2; losing activation stops marking; markers coalesce | yes (exposure + atomic eligibility epoch; coalesced marker) | S P— |
| UNR3 | The unread divider stays until the user leaves the conversation | yes | S |
| UNR4 | Reads on other devices clear local unread and withdraw notifications | yes | S |
| UNR5 | App badge equals unread conversations including muted; failures are a separate marker | yes | S P— |

## ATT — Attention and notifications (model §7.4, §11)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| ATT1 | Behaviour matches the attention matrix for every app state and event class | yes (pure matrix plus exactly-once pre-deferral integration) | S; physical desktop matrix P— |
| ATT2 | One notification per conversation, replaced in place; at most one sound per two-second burst | yes (conversation replacement and two-second sound limiter) | S (pure policy and private daemon contract); physical sound P— |
| ATT3 | Activation opens the chat at its boundary, focuses the window, keeps other drafts; dismissal marks nothing | yes (activation boundary, token/focus request and draft preservation; dismissal marks nothing) | S (private daemon contract); compositor activation P— |
| ATT4 | Withdrawal on exposure, read elsewhere, logout | part (withdraws on exposure/read elsewhere and Quit; logout unavailable) | S (private daemon contract); A— |
| ATT5 | Own messages, receipts, reactions, replayed history never alert | yes (identity/replay/event guards and bounded deduplication) | S (including deferred incoming and same-ID reaction regression) |
| ATT6 | Preview privacy levels, including no avatar below level 1 | yes (three preview levels, avatar suppression) | S (pure policy/private daemon contract); lock-screen P— |
| ATT7 | Mute suppresses mentions unless the exception preference is on | yes (local mute and explicit mention exception) | S; server mute remains unknown, A— |
| ATT8 | Settings shows notification availability, a user-initiated test notification, and never claims success when the daemon is absent | yes (availability, explicit test action and local failure feedback) | S M (private bus unavailable and virtual desktop); host daemon P— |
| ATT9 | Inactive window with new unread raises the task-bar attention hint | yes (inactive unread requests window attention) | S; compositor task-bar hint P— |

## IN — Input and focus (model §8)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| IN1 | Focus order per model §8.2; messages and friend rows are not Tab stops | part (browser/history lists remain single Tab stops; New chat and Stickers join the forward/reverse cycles; full layer traversal S7) | S M |
| IN2 | Selection and focus are visually distinct | yes | M |
| IN3 | Escape closes only the topmost layer; never the window; never cancels an in-flight pairing without confirmation | yes (top layer only; busy sign-in requires confirmation) | S M (layer escape/focus checks); physical input P— |
| IN4 | Layers restore focus to their opener | yes | M |
| IN5 | Modal layers block shortcuts, read markers, and input beneath them; panels do not | yes ([C1 resolved](ux-sessions.md#resolved)) | S M (private virtual-KWin --layers) |
| IN6 | Incoming data and send completion never move focus | yes | S |
| IN7 | Global shortcut table from model §8.1 is implemented and listed in Settings | part (browser/composer/history shortcuts and help implemented; Ctrl+F uses browser search, in-chat search remains unavailable) | S M; physical IME P— |
| IN8 | Scrollbar drag keeps the thumb under the pointer with physical input | part | M P— |

## FB — Feedback and wording (model §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| FB1 | No general status line; each failure appears at its model §9.1 location | yes ([C2 resolved](ux-sessions.md#resolved); P7 transient failure at the message row) | S M (history/local feedback; Settings recovery compiled and synthetic) |
| FB2 | Connection and authentication use one header strip with automatic reconnection and Sign in again; it never starts pairing by itself | yes | S M |
| FB3 | No permanent explanatory captions; no implementation vocabulary in production UI | yes (6a: composer caption removed; sign-in, strip, failed-send and empty-state wording follows [visual design](visual-design.md)) | S M; offline screenshot matrix |
| FB4 | Button labels are verbs of at most three words | yes (short action labels including Retry/Edit/Resend/Replace/Append) | S M (S6 recovery flows) |
| FB5 | Technical diagnostics never appear in message content | yes | S |

## VIS — Visual, theme, scale (model §10)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| VIS1 | Type scale derives from the text-size setting; no hard-coded text sizes except the two minimums | yes (S1) | S M |
| VIS2 | Semantic colours follow the system light/dark scheme using the visual-design token table | yes (6a; only tokens read Palette) | S M; six headless size/theme variants; physical theme V pending |
| VIS3 | Hover, selected, focused, pressed, disabled are distinct and not colour-only | yes (6a controls, focus rings, neutral selected rows and 40% disabled opacity) | M; forward/reverse focus groups |
| VIS4 | One icon family; rail, toolbars, rich cards and delivery use icons, not text glyphs | yes (6a; existing SVG stroke family extended) | S M; detail/profile stack deferred to 6b |
| VIS5 | 100 %, 150 %, 200 % scale with long names, unbroken links, Japanese and emoji text remain usable | part | M (1.5 only) P— |
| VIS6 | Avatars 44 px in chat/friend lists, 32 px in headers/group runs; circular, first-grapheme fallback on stable id tint | yes (6a; eight muted hues); member/profile layouts remain 6b | S M; avatar bounds and sender grouping checks |
| VIS7 | Emoji render in colour on the desktop build or the limitation is recorded | yes (limitation recorded: Unicode font emoji are monochrome in software captures; LINE image-emoji assets retain their colours) | M; physical colour rendering P— |

Phase 6a follows [visual-design.md](visual-design.md) outside the detail/profile
stack (§3.4, phase 6b). Headless MCP captures are in `target/phase6a/`: 980×720,
1280×900 and 420×760, each light and `--demo-dark`, including history, browser,
rich cards, Settings, connection strip, viewer and sign-in. Screenshot review
corrected header alignment, composer divider, viewer gradient placement,
friend counts, redundant rich/photo labels and strip width.

Renderer/API accommodations: the scrollbar hides instantly after one second;
software-renderer shadows are omitted (`draw_box_shadow` is unimplemented in
1.18.1); fixed dialog/column widths clamp to the window; the QR card appears only once
QR data exists. Context menus retain Slint's native styling and behavior:
1.18.1's public `Menu` API has no per-item colour, radius or spacing hooks, and
the compiler rejects imports of its internal menu components. Consequently
menu padding, hover/focus treatment and destructive-item colour remain native.
Photo corner masks compensate for rectangular software clipping without changing
the cached image or original export. No custom font or phase 6b profile layout is introduced.

## OS — Desktop integration (model §11)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| OS1 | App id, class, desktop entry, icon consistent; entry has SingleMainWindow, StartupNotify, Keywords | yes | S; P— |
| OS2 | Single instance: second launch activates the first; D-Bus `org.freedesktop.Application` implemented | yes (activation request and attention fallback) | S; P— |
| OS3 | Notifications via `org.freedesktop.Notifications` with desktop-entry hint and actions | yes (desktop-entry hint, actions and activation tokens) | S (private notification daemon); host desktop P— |
| OS4 | Badge via LauncherEntry | yes (coalesced D-Bus worker) | S P— |
| OS5 | Window activation from a notification works on Wayland, or falls back to the attention hint | yes (token activation with attention fallback) | S (contract and dispatch); physical Wayland activation P— |
| OS6 | State, data, and cache in XDG directories; nothing under the repository by default | yes (XDG defaults; explicit developer path overrides) | S |
| OS7 | Background mode and autostart are off by default and explicit preferences | yes (explicit opt-in preferences, verified tray host, managed autostart) | S (private tray host/lifecycle/autostart); physical tray P— |
| OS8 | System theme, accent, font, and per-monitor scale are followed | yes (Palette roles, default font and toolkit scale) | S M; physical per-monitor checks P— |
| OS9 | Links open through the default handler after activation | yes ([T8 resolved at 05cd26d](ux-sessions.md#resolved)) | S M (demo stub); P— |
| OS10 | File picking and drops use the portal file chooser and `text/uri-list` | yes (zbus OpenFile multiple, URI clipboard and batched winit drops; no GTK) | S; headless path injection M; physical portal/drop P— |
| OS11 | Accessibility tree exposes roles, labels, and states for every control | part | M |
| OS12 | Quit flushes drafts and stops sync; logout withdraws notifications and badge | part (Quit flushes state, joins sync and withdraws attention; logout flow unavailable) | S; desktop withdrawal P— |

## WIN — Windows and lifecycle (model §3.2, §4.4)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| WIN1 | Settings applies local changes immediately; account edits use Save/Cancel with states | part (local text/composer/notification preferences; account edits unavailable) | S M (text/composer and private notification/background harnesses); host desktop P— |
| WIN2 | Unsupported settings are absent, not disabled-looking-saved | yes | M |
| WIN3 | Restart restores selected chat and drafts without opening a modal or marking anything read | yes | S |
| WIN4 | Reconnecting, offline, and expired authentication are distinct states with history and drafts available | yes ([T6, D4 resolved](ux-sessions.md#resolved); core model phase 2) | S; live A— |
| WIN5 | Logout explains the fate of drafts and cache; account switching never shows another account's data | part (switch isolation yes; no logout) | S |
| WIN6 | Media viewer: Escape returns focus to the originating item; wheel and keyboard zoom; drag pan; Save; prev/next scope | yes (viewer keys/wheel/pan, Save and loaded-photo navigation) | M (expanded private virtual-KWin 9452 --layers); physical chooser P— |
| WIN7 | Group info → members → profile shares a controller stack bound to its chat; Back/Escape pop and × closes with opener restoration | yes (Phase 6, U2/U4/U6) | M (offline private --layers/--friends, default + --wide); live unverified |

## SC — Required interaction scenarios

Run with Japanese text, emoji, long names, and mixed media. Physical input
is required where marked P.

| ID | Scenario | Required outcome | Kind | Status |
| --- | --- | --- | --- | --- |
| SC1 | Reading halfway; three arrivals; an image above finishes | Visible text stays put, selection survives, Latest shows 3, unseen arrivals stay unread | M | passed (count and anchor); unread part blocked |
| SC2 | At latest; burst with text and media | Latest content visible, media geometry stable, no bounce or focus theft | M | passed |
| SC3 | Drag scrollbar while a page and new messages arrive | Thumb under pointer; release leaves the intended area visible | P | — |
| SC4 | Load older, fail once, retry, reach the beginning | No anchor jump or duplicates; local failure; end state stops loading | M | PASS S10: 1,000 messages, 20 pages and 20 stable anchors |
| SC5 | Switch chat during load and send, type a newer draft, receive old results | Correct content and draft; status stays with its chat; focus stays | S | passed |
| SC6 | Submit, lose confirmation, restart, reconnect with a matching server message | Uncertain survives restart and reconciles to one item without resending | S | passed synthetically (persisted encrypted-envelope binding, recent catch-up/late-echo reconciliation and Read-alias preservation; legacy rows without request identity remain unresolved) |
| SC7 | Select multiline text with emoji; receive reactions and media | Selection survives; clipboard has text and line breaks only | M | passed |
| SC8 | Search hides the selected chat; results reorder; clear search | Open chat and draft intact; focused result predictable; offset restored | M | passed |
| SC9 | Jump to an old reply or search match, then return | Target highlighted; original anchor recoverable | M | reply jump/return supports local and fetched originals; search targets not implemented |
| SC10 | Receive while active, inactive, minimised, muted, reconnecting | Matches the attention matrix; no burst, no focus theft | P A | blocked (P1, ATT) |
| SC11 | Activate a notification while composing elsewhere | Destination opens at its boundary; prior draft survives; dismissal marks nothing | P A | blocked |
| SC12 | Resize at 200 % with a long draft and media | Independent scrolling; caret, Send, metadata usable | P | — |
| SC13 | Compose Japanese; switch focus; cancel candidates; click the open chat | No accidental send, duplicate commit, stuck preedit, lost draft | P | — |
| SC14 | Open menu, viewer, panel, settings; dismiss in turn | Only intended layers close; focus returns; shortcuts beneath do not fire | M | partial (menu Escape, viewer Escape, settings Escape) |
| SC15 | Launch the app twice; click a notification; log out | One window; it is raised; notifications and badge cleared | P | blocked (physical raise; OS3–OS4) |
| SC16 | Switch the desktop to dark theme while running | Every surface follows; text stays legible | P | — |

## Verification log

Keep account contents and private screenshots out of Git. Newest first.

| Date | Build | Covered | Environment | Result |
| --- | --- | --- | --- | --- |
| 2026-10-19 | basic chat flows phase 7 | P2, editable T5, new chats; MSG4/MSG10–11, CMP11–12, OUT1/OUT3, IN1 | Offline workspace; release ui-debug headless software; fresh private instances | PASS: fmt, workspace/all-target Clippy with warnings denied (including ui-debug), workspace tests (195 passed, 5 ignored), release ui-debug build. Base plus every existing group passes; layers, focus, friends and scenarios also pass with --wide. New --compose passes react/cancel, owned-package image picker/send/Escape/cached reopen, typed first send to a friend without a chat, and group creation. Three core flows cover reaction set/cancel/failure rollback and op 139 dedup, sticker echo dedup/retry, and local friend/group success/failure. Inspected all three surfaces and the selected-group variant in light/dark at 980×720 and 420×760; corrected popover anchoring, narrow picker width, tabs/images and dialog controls. Synthetic data and loopback only; no real account, external network or Git operations. All new LineBackend writes remain **live unverified**. |
| 2026-10-18 | profiles phase 6 | U0–U6, traced P5/P6, SH8, WIN7; 6a viewer/rail/preview leftovers | Offline workspace; release ui-debug software renderer; fresh private instance for every group at 980×720 and 1280×900 | PASS: fmt, all-target Clippy with ui-debug and warnings denied, workspace tests (192 passed, 5 ignored), release ui-debug build. All 11 primary groups (base, unread, scenarios, friends, layout-photos, reading, rich, media, auth-recovery, layers, focus) pass at both sizes (22 fresh instances). Members/action checker passes at 420×760, default and wide sizes, including Rename/Add friend/mute/Leave, invitees and read-only draft; layers also checks avatar-viewer opener restoration. 420 px capture inspected: round avatars and selected rail icon without a left bar. Synthetic data and loopback fixtures only; no real account, external network or Git operations. Traced writes remain **live unverified**; Block, Report, Invite, accept/reject invitations and remove member remain untraced. The separate legacy composer checker still has stale pre-6a recovery assertions (V6), outside these primary groups. |
| 2026-10-18 | protocol phase 5 | T5, P1, P7; RD3/RD8, MSG4/MSG7, FB1 | Offline workspace; release ui-debug headless software, ten fresh private instances via run-ui-debug.py + check-ui-mcp.py | fmt, Clippy with warnings denied, workspace tests (197 passed, 6 ignored), and ui-debug release build pass. scenarios, reading, rich, layers, friends, focus, layout-photos, media, unread and auth-recovery all pass. Layers adds OA thumbnail/original, live reaction, Unsend menu/dialog, Escape/Cancel, failure feedback and success. Synthetic data and loopback fixtures only; live unverified. T9 remains blocked and crypto is unchanged. |
| 2026-10-18 | core model phase 3 | Media LRU, cleanup, all ten UI check groups | Offline release workspace; ui-debug headless software, fresh private instance per group via run-ui-debug.py | fmt and all-target clippy (`-D warnings`, including ui-debug) pass; 194 tests pass, 6 ignored. scenarios, reading, rich, layers, friends, focus, layout-photos, media, unread and auth-recovery all pass. Synthetic data only. |
| 2026-10-18 | T11 working tree after `d0097e7` | Session credentials sealed with a Secret Service key; plaintext development store | Offline synthetic Rust tests; `dbus-run-session` with a synthetic org.freedesktop.secrets (`LINE_DESKTOP_TEST_BUS=1 ... -- --ignored`) | PASS: formatting, all-target Clippy with warnings denied, workspace tests (136 desktop, 121 library; ignored prerequisites unchanged plus the new private-bus test). Four credential tests (sealing hides tokens, key reuse, tamper rejection, verified one-time migration, no downgrade, refresh kept without a keyring, development store) and the private-bus contract (locked collection prompts, item attributes, relock) pass. Host keyring and the account `session.json` not touched. |
| 2026-10-17 | V1 review fixes; debug `fddbc0c9cf7c` | Scenario access, modal focus/closing, rich tree bound, file-free layer tests | Private virtual KWin MCP port 9481; a fresh owned scope for each check; software snapshots at 980×720 and 360×520 | PASS: base and each UI flag separately (`--unread`, `--paging`, `--scenarios`, `--friends`, `--auth-recovery`, `--layout-photos`, `--reading`, `--rich`, `--layers`), browser and composer scripts. F12/title hint replaces the floating opener; Close/Escape restores the prior focused item; scenarios close before running and start with persistent shell focus. No scenario-dialog PPM writes during tests. Formatting, normal release build, workspace tests (74 desktop + 32 library; 4 prerequisite tests ignored) and all-target Clippy with warnings denied pass. Inspected profile, conversation and connection-notice MCP snapshots plus a headless transient-notice snapshot under `target/v1-review-snapshots/`; profile Close click and Ctrl+Q after dialog Escape pass. Only wrapper-reported scopes stopped; port released. Physical/account gates unchanged. |
| 2026-10-17 | `d1e5db5` source; earlier runtime; debug `35cd9f05ef21` | S7 layers, S9 attention, S12 background; final source gates | Offline software renderer; private D-Bus daemon contracts and private virtual KWin scopes 9452/9472 | Formatting and Clippy pass. Workspace: 73 desktop and 32 library tests pass (3 desktop private-bus tests and 1 library WASM test ignored in normal suite); all three desktop private-bus tests pass separately. Expanded layers MCP passes viewer/profile/focus/draft checks. Attention matrix, privacy, daemon availability, exact-once deferred events, tray host loss, close/Open/draft/Quit and managed autostart tests pass. Background wrapper isolates host display/bus, cleans owned processes and refuses stale ownership. Physical IME, compositor activation, sound/DND/lock-screen/tray and account gates remain unverified. |
| 2026-10-17 | S10 working tree, debug `df19a182` | RD6, RD7, RD14, SC4; bounded incoming/deferred state and alias anchors | Offline synthetic Rust tests; exclusive MCP 9460 scoped process; Wayland software renderer, 980×720 logical, scale 1.5 | Seven targeted controller tests and four pure range tests pass; SC4 traverses 1,000 messages in 20 pages with 20 stable prepend/failure/retry anchors; beginning stops loading. UI ≤200, retained ≤600 per view, eight total views. RSS 62,056→71,100 KiB. Snapshot inspected. Collective workspace suite passes (73 desktop, 32 library); all three private-bus desktop harnesses pass separately. Account and physical V remain unverified. |
| 2026-10-17 | `d1e5db5` | S1–S12 integration; SC4 and core offline flows | Workspace release/offline tests, all-target Clippy, inspected 980×720/15 and 360×520/24 snapshots; isolated background virtual KWin MCP 9461 | Formatting and Clippy pass; 73 desktop and 32 library tests pass, three private-bus desktop tests additionally pass, one WASM prerequisite remains ignored. Core scenarios, photos/grouping, friends, auth recovery and SC4 pass (`/tmp/line-final-mcp.log`): 1,000 messages over 20 pages, 20 stable anchors, UI ≤200/retained ≤600, RSS 68,936→71,852 KiB at scale 1. Owned process/compositor exited and port closed. Physical/account verification remains separate. |
| 2026-10-17 | `3639708` + S8 worktree | Unread counts/staleness, eligibility guards, 120-unread paging boundary, 300 cached rows, full-queue revocation, precise read ranges | Offline synthetic controller/library tests, Slint headless software renderer | 9 S8 tests and atomic revocation test pass; read-range sentinel test passes. No account request, live marker, physical badge, IME or compositor verification. |
| 2026-10-17 | S11 working tree; MCP `241b3ed` debug artifact | MSG1/2 rich selection mode, MSG6–MSG8, SC9 retained reply | Slint 1.18.1 software; native Wayland 1.5 scale; headless 980×720/15, 360×520/24 and 360×1100/24 | Four rich tests pass, including native 2048-run cold render and narrow large-text reflow (911 ms including render/I/O). `--rich` MCP passes measured wrapping/image position, fallback/newline clipboard, exact reply return anchor, metadata and synthetic file Save bytes. Snapshots inspected; narrow Reply overflow fixed. Zero-width initial layout is guarded by the stress test. Contextual shaping, cross-run selection, animated emoji and >2048-run fallback limits remain explicit above. No account access. |
| 2026-10-17 | S6 working tree after `3639708`; MCP binary `33547916` | CMP1–CMP2 guard, CMP5, CMP8 text, OUT2 failure marks, OUT5–OUT7 | Three focused synthetic controller tests; Wayland 1.5× Slint 1.18.1 software renderer, private-bus demo MCP 9457 | Japanese/astral caret, selection and long-draft scroll restoration, unchanged-chat guard, persisted preference/request sequence, both failure outcomes, edit decisions and late original echo pass synthetic checks. MCP verifies multiline text, Escape, Enter preference/reopen, Shift+Enter, Sending/Sent/Not sent/Unconfirmed, Retry, Replace/Append/Cancel, duplicate Resend warning and newer-draft retention; inspected composer screenshot shows no text clipping. MCP Ctrl+Q exits the demo cleanly (status 0). Combined source regression suite passes in the final collective run; physical Fcitx remains P—. No account access. |
| 2026-10-17 | S4 working tree after `3639708` | Category filters, row markers, outgoing previews, relative dividers, list and global keyboard | Offline software renderer; MCP scale 1.5; headless 360×520/24 | Browser unit tests 2/2 pass including narrow visible Tab path. Dedicated `check-browser-mcp.py` passes category/draft/Ctrl+1/2/Alt+Up/Down/Ctrl+F/Home/End/Enter; default MCP draft checks pass. Normal/small `/tmp/line-s4*.png` and dark `/tmp/line-s4-mcp.png` inspected. Calendar boundary tests 2/2 pass in isolation against cached chrono. Final collective source gates and core MCP pass; see the final log entry. No physical/account checks. |
| 2026-10-17 | `3639708` S3 | Rail, compact Back, LINE title, local feedback, wording and Settings help; required workspace checks, snapshots, full MCP | Offline Slint software; MCP scale 1.5 | Combined checkpoint: 40 tests pass, fmt and clippy pass; `/tmp/s1-final.png`, `/tmp/s1-final-small.png` inspected; all required MCP flags pass (`/tmp/s1mcp.log`). Physical Back/IME checks remain unverified. |
| 2026-10-17 | S1–S3/P1–P4 working tree | VIS1, VIS2, SH4 partial; modular UI/controller regression | Slint 1.18.1 software renderer; offline snapshots 980×720/15 and 360×520/24; Plasma Wayland BreezeDark at 1.5 scale; MCP | Required workspace tests and Clippy with warnings denied pass; formatting passes. Required MCP batch (`--scenarios --layout-photos --friends --auth-recovery`) passes. Snapshots and actual dark desktop screenshot inspected; caption/time geometry scales at 24 px. Shared Pane covers chats/friends/history; standard editor and viewer/profile/auth sheet scrollbars remain. Physical IME/pointer/scale V and account checks remain unverified. |
| 2026-10-17 | S5 working tree; debug SHA `241b3ed17333`, captured 01:12:05 UTC | RD7 recent strip, RD13, MSG3–MSG5 partial, MSG7 placeholders, MSG9, IN1 reading, OS9 demo | Offline Rust tests; Wayland/software Slint 1.18.1 MCP port 9455, scale 1.5, dark theme | Four URL/kind/label tests pass. `--reading` passes: four Tabs search→composer, reverse history traversal, arrows/Home/End/pages, body first-arrow scroll, click focus retains following, stable loading y, full URL inspection, drag selects/copies without activation, native-menu Copy link, demo activation, twelve kinds, selected-body chat switch without reentrant borrow. Top/bottom screenshots inspected. Links remain separate action runs; full native-menu mutation deferral, physical input/default-handler dispatch and account checks remain unverified. |
| 2026-10-17 | S2 working tree | OS1, OS2, OS6 | Offline synthetic Rust harness, private `dbus-run-session`, isolated installer staging | 5 activation/path/migration/lock tests and isolated Clippy pass. Private bus verifies primary ownership, second-launch Activate/exit, introspection, Open, and open-chat dispatch. A copied release binary passes two offline demo launches: owner PID retained, second exits 0; compositor focus is not asserted. Normal and dev-paths staged entries/service pass field checks. Physical compositor raising and notification activation remain unverified; no account access. |
| 2026-10-17 | P4 working tree | Rich-content protocol parser (enables MSG6–MSG8) | Offline synthetic Rust harness importing `desktop/rich.rs` | 4 tests and Clippy with warnings denied pass: UTF-16/UTF-8 boundaries, malformed and overlapping spans, metadata numbers, reply relation filtering, membership. No account access or rich UI verification. |
| 2026-10-17 | `fc4c112` | Workspace tests; headless snapshots 980×720/15 px, 360×520/24 px for demo, groups, friends, photos | Linux, Slint 1.18.1 software renderer, offline | 18 tests pass, 1 WASM test ignored. Snapshots show the defects listed in ux-sessions.md (full-window sheets, button-like friend rows, fixed caption sizes at 24 px, Latest control visible at first open in headless mode). No desktop run. |
| 2026-10-16 | `fc4c112` | NAV1, NAV2, NAV11–NAV14 partial; friends MCP run | Wayland, scale 1.5, `check-ui-mcp.py --friends --scenarios --layout-photos --auth-recovery` | Passed. Friend and group retrieval against the account not exercised. |
| 2026-10-15 | `f7ab75a` and follow-ups | RD2–RD5, RD8 partial, MSG1–MSG3, OUT1, OUT3, OUT4, OUT8, CMP3–CMP7, FB2, SC1, SC2, SC5–SC8 | Wayland, scale 1.5, MCP simulated pointer and keyboard; `cargo test` | Passed as simulated input. Physical pointer, Fcitx, and 100/200 % scale never run. Sticker fallback, group identity, photo viewer, and authentication recovery sheets checked by MCP. |
| 2026-10-15 | `f7ab75a` | Idle resource sample | Wayland, release build, 980×720 | 37.9 MiB RSS, 0.4 % CPU idle; scale factor not recorded. |
| before 2026-10-15 | earlier | Real account: chat list, recent messages, photos, stickers, paid reactions displayed; one real send not yet validated | Native session, since invalidated by the Chrome login | Historical; does not prove current authorization. |
| 2026-10-18 | Phase 6a working tree | VIS1–VIS4, VIS6, FB3; visual-design implementation outside §3.4 | Offline fmt, all-target workspace Clippy with warnings denied, workspace tests, release ui-debug; fresh owned headless MCP scopes | PASS: 199 tests, 6 ignored prerequisites; fmt/Clippy/build pass. Default, unread, scenarios, friends, layout-photos, reading, rich, media, auth-recovery, layers and focus pass separately at 980×720; layers/focus/friends/scenarios also pass separately at 1280×900. Six size/theme variants (980×720, 1280×900, 420×760; light/dark) captured through MCP and inspected in `target/phase6a/`. Native-menu styling and absent software shadows are recorded above. No account, external network, Git commands or physical-input verification. |
| 2026-10-18 | Phase 4 known defects | MSG5, CMP7; continuous send linking, first-identity local work, C6/C7, S23/S24 and wide layouts | Offline workspace tests, formatting, all-target Clippy with warnings denied, release ui-debug; fresh owned headless demos | 196 tests pass (6 prerequisite-dependent tests ignored). At 980×720, unread, scenarios, friends, layout-photos, reading, rich, media, auth-recovery, layers and focus pass separately; at 1280×900, layers, focus, friends and scenarios pass separately with profiles as side panels. No account, external network or physical-input verification. |

| 2026-10-19 | Signed-out follow-up working tree | Retryable backend open failure; typed crypto/keyring prerequisites; immediate pairing cancellation with generation fencing | Offline release workspace tests, fmt, all-target Clippy with warnings denied, release ui-debug; fresh private headless demos at 980×720 | PASS: 209 tests, 5 ignored prerequisites. Core regressions cover open failure/retry after repair, cancellation before pairing and during initial open, stale completions, and disconnected job-channel failure. Signed-out checks include a fresh missing-module backend, path-specific startup/retry messages and no QR. Default, auth-recovery, scenarios, reading, rich, layers, friends, focus, layout-photos, media, unread and compose groups pass separately. Logs: `/tmp/line-signedout-verified-*.log`, `/tmp/line-signedout-ui*.log`. No real account, external network or Git commands. |
| 2026-10-19 | Phase 8; release ui-debug SHA `b1389d00d4e6` | P3, CMP8, CMP9, OS10 | Offline workspace tests, fmt, all-target Clippy with warnings denied, release ui-debug; fresh owned headless demos | PASS: 199 tests, 5 prerequisite-dependent tests ignored. All primary check groups pass separately at 980×720, plus layers/focus/friends/scenarios separately at 1280×900. Compose checks pass in light/dark at 980×720 and 420×760: automation-only path injection opens the dialog, removes one item, sends ordered rows, observes nonzero upload progress and waits for sent rows. Dialog/uploading screenshots inspected in all four variants; progress aligns to the thumbnail and grows from the left. Independent media fixture also verifies encryption against the received envelope. No account, external network or Git commands; physical portal/clipboard/drop and live sends remain unverified. |

| 2026-10-19 | Signed-out/attachments merge resolution | Explicit session state with independent upload workers, cancellation and generation fencing; typed errors and late send confirmation | Offline workspace tests, fmt, all-target Clippy with warnings denied, release ui-debug; private headless demos at 980×720 | PASS: 216 tests, 5 prerequisite-dependent tests ignored. Default, compose, signed-out (including missing-module recovery), and auth-recovery check groups pass. Regression covers authorization loss during a blocked upload, recovery before completion, and retry; signed-out attachment intents are ignored. Logs: `/tmp/line-merge-{clippy,tests,build}.log`, `/tmp/line-merge-ui-*.log`. No real account, external network or Git commands. |

Detailed narratives of the earlier follow-ups (identity-preserving model
edits, encrypted state storage, authentication recovery, newest-first
sorting, sticker and delivery status slots, group identity) are in the Git
history of this file before 2026-10-17 and in app-design.md.

| 2026-10-17 | final debug SHA `35cd9f05ef21` | S8 scrolled incoming/unread/divider | Owned private virtual KWin MCP `9478`, no account or host focus request | PASS: current 3, other chat 2, stable anchor, Latest and persistent divider; owned app/scope gracefully closed. Physical badge/focus remains P—. |
| 2026-10-19 | Real-app feedback fixes | Message-row React hover/focus and fixed slot; hidden unread tail; avatar alpha; resolved chat events | Offline workspace; release ui-debug headless software | PASS: fmt, all-target workspace Clippy with ui-debug and warnings denied, workspace tests (225 passed, 5 ignored), release ui-debug build. All default, unread, scenarios, friends, layout-photos, reading, rich, media, auth-recovery, layers, focus, compose and signed-out groups pass; layers/focus/friends/scenarios also pass with --wide. Signed-out includes missing-module recovery. Inspected 980×720 light/dark group screenshots in `target/review2/`; pointer bubble→gap→React→away and focused-message Tab→React→Enter pass in both themes. Hidden-only official-account demo and read-exposure regression cover the unread marker blind spot. Existing local WASM fixture copied into ignored artifacts for offline tests. No real account, external network or Git commands. |
