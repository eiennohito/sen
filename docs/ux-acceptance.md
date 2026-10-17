# Native client UX acceptance

Status as of 2026-10-17, commit `fc4c112` plus this documentation change.
Requirements derive from [ux-model.md](ux-model.md); section numbers refer to
it. Implementation order lives in [ux-sessions.md](ux-sessions.md).

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

Current totals: 120 requirements. Impl `yes` 32, `part` 39, `no` 47, one
not applicable, one unknown. Nothing has physical or account verification.

## Blockers

- **Device slot.** LINE allows one Chrome-type secondary device. Chrome holds
  it; native pairing would sign Chrome out. All `A` verification waits for
  the user to assign the slot to the native app.
- **No live sync.** The worker has no event stream. Every requirement in
  §UNR and §ATT, and the live parts of §RD and §OUT, depend on the protocol
  sessions P1–P3 in ux-sessions.md.
- **Physical verification** needs a person at the desktop: Fcitx, scrollbar
  dragging, 100/150/200 % scale, dark theme, notifications.

## SH — Shell and layout (model §3, §10)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| SH1 | Rail with icon destinations; current destination shown as selected, not disabled; Settings and account at the rail bottom | no | — |
| SH2 | Browser pane, conversation pane, and detail panel scroll independently; wheel input never spills across panes | part (two panes; no panel) | M |
| SH3 | Header and composer stay visible while history scrolls | yes | M |
| SH4 | One scrollbar style across panes; thumb ≥ 10 px; appears when content overflows; no text rewrap when it appears | part (three styles) | M |
| SH5 | Window and pane resizing keep selected chat, draft, focus, and reading anchor | part | S |
| SH6 | Narrow layout below 740 px: Back in header returns to the browser at its previous offset; unread reachable from the conversation pane | part (Back exists; no unread) | M |
| SH7 | Minimum 360×520 at the largest text size has no overlapping controls | part (verified at 24 px only by snapshot) | M |
| SH8 | Settings is a dialog in wide layout; profile and members are panels; only sign-in and the viewer are sheets in wide layout | no (all are full-window sheets) | — |
| SH9 | Window title is "LINE" (optionally with unread count), never the account name | no | — |

## NAV — Conversation list, friends, search (model §3, §4.2, §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| NAV1 | Switching destinations keeps the open conversation and draft | yes | S M |
| NAV2 | Each destination keeps query, filters, expansion, focused result, and offset for the session | yes | S M |
| NAV3 | Chat row: avatar, name, time, one preview line, unread badge, mute and failed-send markers, draft marker; metadata never covers name or preview | part (no badge or markers) | M |
| NAV4 | Outgoing preview is prefixed; previews are single-line; times follow model §9.2 | part (no prefix; `M/D` and `HH:MM` only) | S |
| NAV5 | Category filter All / Friends / Groups / Official (as the protocol allows) without closing the open chat | no | — |
| NAV6 | Arrivals reorder rows without moving one under an active press or menu; the visible anchor is preserved; selection kept by identity | yes | S |
| NAV7 | Opening a chat keeps unread information visible until content is exposed | no | — |
| NAV8 | Loading more conversations keeps the list offset; loading, end, failure, retry are distinct; retry never duplicates rows | part (no end state) | S |
| NAV9 | Refresh keeps cached rows and selection; progress and errors are local to the list footer | part (error row exists; status line also used) | S |
| NAV10 | Search scope is stated in the placeholder or empty state, not as a permanent caption | no (permanent caption) | — |
| NAV11 | Up/Down move through results, Enter opens, Escape clears, all gated on IME composition; clearing restores the prior offset | yes | M |
| NAV12 | Filtering may hide the open chat without closing it or touching the draft | yes | S M |
| NAV13 | Friends list uses list rows with section headers, not buttons; a click opens a profile panel; nothing sends | part (rows are centred buttons; profile is a sheet) | M |
| NAV14 | Friends load and refresh failures keep loaded rows and show a local retry | yes | S |
| NAV15 | Alt+Up/Alt+Down switch conversation; Ctrl+F focuses search; Ctrl+1/2/3 switch destinations | no | — |

## RD — Reading and history (model §5)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| RD1 | First open of a read chat shows the bottom; first open of an unread chat shows the unread divider with context | part (bottom only) | M |
| RD2 | Reopening within the session restores the previous anchor and at-latest state | yes | S M |
| RD3 | Incoming messages move the view only while at latest; receipts, reactions, media, and previews never move it | yes | S M |
| RD4 | Away from latest, a Latest control shows the count of incoming messages appended below the viewport; older loaded messages do not count | yes | M |
| RD5 | Latest and Send go to the bottom and resume following; typing, confirmations, and receipts do not | yes | M |
| RD6 | Older history loads before the top is reached, one request per boundary, stops at the beginning | no | — |
| RD7 | Loading, failure with Retry, and beginning-of-history are items inside the history that do not shift content | no (loading row above history shifts content) | — |
| RD8 | Media load, failed media, font change, reaction change, and withdrawal keep the anchor; a vanished anchor falls back to its neighbour | part | M |
| RD9 | Switching chats during a load never flashes the previous chat or moves the new chat | yes | S |
| RD10 | Dragging a scrollbar or selecting text suspends following; release resumes only if at latest | part | M (simulated) P— |
| RD11 | Date dividers (Today, Yesterday, weekday, date) and five-minute sender groups per model §9.2 | part (raw dates) | S M |
| RD12 | Bursts do not bounce the viewport; any animation is interruptible and honours reduced motion | yes (no animation) | M |
| RD13 | History is one Tab stop; arrows, Home, End, PageUp, PageDown scroll it; messages are click-focusable only | no (each message is a Tab stop; only PageUp/Down through an unreachable focus scope) | — |
| RD14 | The loaded window is virtualised or windowed before exceeding a few hundred rows | no | — |

## MSG — Message content and actions (model §6, §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| MSG1 | Pointer and keyboard selection inside a message, Copy without entering an editor | yes | M |
| MSG2 | Copy yields visible text with line breaks and emoji fallback, no timestamps or labels | yes | M |
| MSG3 | Right click keeps an existing selection and targets the message; incoming updates keep selection and menu | part | S M |
| MSG4 | Context menu offers Copy selection, Copy message, Copy link, Open photo as applicable; menus stay on screen | part (copy only) | M |
| MSG5 | Links are detected, shown distinctly, inspectable (full URL), and opened only on deliberate activation | no | — |
| MSG6 | Reply previews jump to the referenced message with highlight and a return path | no | — |
| MSG7 | Photo, video, voice, file, location, contact, sticker, system, withdrawn, undecryptable, and unknown content each have a distinct placeholder or presentation; one bad message hides nothing | part (photo, sticker, video/voice/location labels; one generic "Unsupported") | S |
| MSG8 | Inline LINE emoji render as runs within text | no | — |
| MSG9 | Accessible label of a message includes direction, sender, time, and delivery state | part (direction only) | — |
| MSG10 | Stickers have no bubble; photos keep aspect ratio in a reserved frame; reactions sit under the bubble | yes | M |

## CMP — Composer (model §8.1, app-design IME checks)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| CMP1 | Enter inserts a newline by default; Ctrl+Enter sends; Shift+Enter always newline; "Enter sends" is a preference | part (no preference; Shift+Enter untested) | S M |
| CMP2 | No shortcut fires during IME composition; Fcitx preedit, candidates, cancel, and refocus pass the app-design table | part (guard exists) | S P— |
| CMP3 | Composer grows to one third of the pane then scrolls; caret and last line visible; Send stays reachable | yes | M |
| CMP4 | Send is disabled with a visible reason for whitespace, no chat, offline, storage failure, over-size | yes | S |
| CMP5 | Switching chats keeps text, caret, selection, and scroll per chat; clicking the open chat changes nothing | part (text and scroll only) | S M |
| CMP6 | Drafts survive restart in private storage; nothing moves between chats or accounts | yes | S |
| CMP7 | After submit the composer is empty, focused, and a late result cannot alter newer text | yes | S M |
| CMP8 | Pasting text keeps line breaks; pasting or dropping media opens the attachment preview | part (text only) | M |
| CMP9 | Attachment selection shows a preview with destination, remove, cancel, Send; nothing transmits on selection or drop | no | — |
| CMP10 | Shortcut help is in Settings, not a permanent caption under the composer | no | — |

## OUT — Outgoing lifecycle (model §6)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| OUT1 | Submit creates a durable Pending item before any network call | yes | S |
| OUT2 | Pending, Confirmed, Read, Rejected, Uncertain are visually and accessibly distinct in a fixed slot | part (no Read, no Rejected) | S M |
| OUT3 | Confirmation updates the same item; refresh, reconnect, restart never duplicate it; Confirmed never implies Read | yes | S |
| OUT4 | The user can keep composing while sends are pending | yes | S |
| OUT5 | Definite rejection is classified separately from an uncertain result | no (every failure is Uncertain) | — |
| OUT6 | Rejected offers safe Retry and Edit; Uncertain offers Resend with duplicate warning and Edit; Edit never overwrites a newer draft | part (Edit only) | S M |
| OUT7 | A send failure does not change the app's connection state or show the connection banner | no (it does both) | — |
| OUT8 | Restart turns Pending into Uncertain, never into a draft | yes | S |
| OUT9 | Receipts update quietly; group reader counts only with protocol support | no | — |

## UNR — Unread and read (model §7.1–7.3)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| UNR1 | Unread counts and mute flags come from the server list and live events; unknown shows as a dot | no (count is requested and discarded) | — |
| UNR2 | Read eligibility follows model §7.2; losing activation stops marking; markers coalesce | no | — |
| UNR3 | The unread divider stays until the user leaves the conversation | no | — |
| UNR4 | Reads on other devices clear local unread and withdraw notifications | no | — |
| UNR5 | App badge equals unread conversations including muted; failures are a separate marker | no | — |

## ATT — Attention and notifications (model §7.4, §11)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| ATT1 | Behaviour matches the attention matrix for every app state and event class | no | — |
| ATT2 | One notification per conversation, replaced in place; at most one sound per two-second burst | no | — |
| ATT3 | Activation opens the chat at its boundary, focuses the window, keeps other drafts; dismissal marks nothing | no | — |
| ATT4 | Withdrawal on exposure, read elsewhere, logout | no | — |
| ATT5 | Own messages, receipts, reactions, replayed history never alert | no | — |
| ATT6 | Preview privacy levels, including no avatar below level 1 | no | — |
| ATT7 | Mute suppresses mentions unless the exception preference is on | no | — |
| ATT8 | Settings shows notification availability, a user-initiated test notification, and never claims success when the daemon is absent | no | — |
| ATT9 | Inactive window with new unread raises the task-bar attention hint | no | — |

## IN — Input and focus (model §8)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| IN1 | Focus order per model §8.2; messages and friend rows are not Tab stops | no | — |
| IN2 | Selection and focus are visually distinct | yes | M |
| IN3 | Escape closes only the topmost layer; never the window; never cancels an in-flight pairing without confirmation | part (Escape aborts pairing immediately) | S |
| IN4 | Layers restore focus to their opener | yes | M |
| IN5 | Modal layers block shortcuts, read markers, and input beneath them; panels do not | part (shortcut guard exists) | S |
| IN6 | Incoming data and send completion never move focus | yes | S |
| IN7 | Global shortcut table from model §8.1 is implemented and listed in Settings | no (Ctrl+Enter only) | — |
| IN8 | Scrollbar drag keeps the thumb under the pointer with physical input | part | M P— |

## FB — Feedback and wording (model §9)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| FB1 | No general status line; each failure appears at its model §9.1 location | no | — |
| FB2 | Connection and authentication use one header strip with Retry and Sign in again; it never starts pairing by itself | yes | S M |
| FB3 | No permanent explanatory captions; no implementation vocabulary in production UI | no | — |
| FB4 | Button labels are verbs of at most three words | part | — |
| FB5 | Technical diagnostics never appear in message content | yes | S |

## VIS — Visual, theme, scale (model §10)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| VIS1 | Type scale derives from the text-size setting; no hard-coded text sizes except the two minimums | no | — |
| VIS2 | Colours come from the system palette and follow light/dark | no (fixed light palette; widgets follow system) | — |
| VIS3 | Hover, selected, focused, pressed, disabled are distinct and not colour-only | part | M |
| VIS4 | One icon family; rail and markers use icons, not text glyphs | no | — |
| VIS5 | 100 %, 150 %, 200 % scale with long names, unbroken links, Japanese and emoji text remain usable | part | M (1.5 only) P— |
| VIS6 | Avatars 40 px in lists, 32 px in groups, circular; bubble widths per model | yes | M |
| VIS7 | Emoji render in colour on the desktop build or the limitation is recorded | unknown | P— |

## OS — Desktop integration (model §11)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| OS1 | App id, class, desktop entry, icon consistent; entry has SingleMainWindow, StartupNotify, Keywords | part (id and icon; entry fields missing) | P— |
| OS2 | Single instance: second launch activates the first; D-Bus `org.freedesktop.Application` implemented | no | — |
| OS3 | Notifications via `org.freedesktop.Notifications` with desktop-entry hint and actions | no | — |
| OS4 | Badge via LauncherEntry | no | — |
| OS5 | Window activation from a notification works on Wayland, or falls back to the attention hint | no | — |
| OS6 | State, data, and cache in XDG directories; nothing under the repository by default | part (cache only) | — |
| OS7 | Background mode and autostart are off by default and explicit preferences | n/a (close quits) | S |
| OS8 | System theme, accent, font, and per-monitor scale are followed | part (scale only) | P— |
| OS9 | Links open through the default handler after activation | no | — |
| OS10 | File picking and drops use the portal file chooser and `text/uri-list` | no | — |
| OS11 | Accessibility tree exposes roles, labels, and states for every control | part | M |
| OS12 | Quit flushes drafts and stops sync; logout withdraws notifications and badge | part (flush only) | S |

## WIN — Windows and lifecycle (model §3.2, §4.4)

| ID | Requirement | Impl | Verified |
| --- | --- | --- | --- |
| WIN1 | Settings applies local changes immediately; account edits use Save/Cancel with states | part (text size only) | M |
| WIN2 | Unsupported settings are absent, not disabled-looking-saved | yes | M |
| WIN3 | Restart restores selected chat and drafts without opening a modal or marking anything read | yes | S |
| WIN4 | Reconnecting, offline, and expired authentication are distinct states with history and drafts available | part | S |
| WIN5 | Logout explains the fate of drafts and cache; account switching never shows another account's data | part (switch isolation yes; no logout) | S |
| WIN6 | Media viewer: Escape returns focus to the originating item; wheel and keyboard zoom; drag pan; Save; prev/next scope | part (buttons, Escape) | M |
| WIN7 | Group and member surfaces identify their conversation; reopening focuses the existing one | no | — |

## SC — Required interaction scenarios

Run with Japanese text, emoji, long names, and mixed media. Physical input
is required where marked P.

| ID | Scenario | Required outcome | Kind | Status |
| --- | --- | --- | --- | --- |
| SC1 | Reading halfway; three arrivals; an image above finishes | Visible text stays put, selection survives, Latest shows 3, unseen arrivals stay unread | M | passed (count and anchor); unread part blocked |
| SC2 | At latest; burst with text and media | Latest content visible, media geometry stable, no bounce or focus theft | M | passed |
| SC3 | Drag scrollbar while a page and new messages arrive | Thumb under pointer; release leaves the intended area visible | P | — |
| SC4 | Load older, fail once, retry, reach the beginning | No anchor jump or duplicates; local failure; end state stops loading | M | blocked (P3) |
| SC5 | Switch chat during load and send, type a newer draft, receive old results | Correct content and draft; status stays with its chat; focus stays | S | passed |
| SC6 | Submit, lose confirmation, restart, reconnect with a matching server message | Uncertain survives restart and reconciles to one item without resending | S | passed (restart to Uncertain; reconciliation with server ID) |
| SC7 | Select multiline text with emoji; receive reactions and media | Selection survives; clipboard has text and line breaks only | M | passed |
| SC8 | Search hides the selected chat; results reorder; clear search | Open chat and draft intact; focused result predictable; offset restored | M | passed |
| SC9 | Jump to an old reply or search match, then return | Target highlighted; original anchor recoverable | M | blocked (MSG6) |
| SC10 | Receive while active, inactive, minimised, muted, reconnecting | Matches the attention matrix; no burst, no focus theft | P A | blocked (P1, ATT) |
| SC11 | Activate a notification while composing elsewhere | Destination opens at its boundary; prior draft survives; dismissal marks nothing | P A | blocked |
| SC12 | Resize at 200 % with a long draft and media | Independent scrolling; caret, Send, metadata usable | P | — |
| SC13 | Compose Japanese; switch focus; cancel candidates; click the open chat | No accidental send, duplicate commit, stuck preedit, lost draft | P | — |
| SC14 | Open menu, viewer, panel, settings; dismiss in turn | Only intended layers close; focus returns; shortcuts beneath do not fire | M | partial (menu Escape, viewer Escape, settings Escape) |
| SC15 | Launch the app twice; click a notification; log out | One window; it is raised; notifications and badge cleared | P | blocked (OS2–OS4) |
| SC16 | Switch the desktop to dark theme while running | Every surface follows; text stays legible | P | — |

## Verification log

Keep account contents and private screenshots out of Git. Newest first.

| Date | Build | Covered | Environment | Result |
| --- | --- | --- | --- | --- |
| 2026-10-17 | `fc4c112` | Workspace tests; headless snapshots 980×720/15 px, 360×520/24 px for demo, groups, friends, photos | Linux, Slint 1.18.1 software renderer, offline | 18 tests pass, 1 WASM test ignored. Snapshots show the defects listed in ux-sessions.md (full-window sheets, button-like friend rows, fixed caption sizes at 24 px, Latest control visible at first open in headless mode). No desktop run. |
| 2026-10-16 | `fc4c112` | NAV1, NAV2, NAV11–NAV14 partial; friends MCP run | Wayland, scale 1.5, `check-ui-mcp.py --friends --scenarios --layout-photos --auth-recovery` | Passed. Friend and group retrieval against the account not exercised. |
| 2026-10-15 | `f7ab75a` and follow-ups | RD2–RD5, RD8 partial, MSG1–MSG3, OUT1, OUT3, OUT4, OUT8, CMP3–CMP7, FB2, SC1, SC2, SC5–SC8 | Wayland, scale 1.5, MCP simulated pointer and keyboard; `cargo test` | Passed as simulated input. Physical pointer, Fcitx, and 100/200 % scale never run. Sticker fallback, group identity, photo viewer, and authentication recovery sheets checked by MCP. |
| 2026-10-15 | `f7ab75a` | Idle resource sample | Wayland, release build, 980×720 | 37.9 MiB RSS, 0.4 % CPU idle; scale factor not recorded. |
| before 2026-10-15 | earlier | Real account: chat list, recent messages, photos, stickers, paid reactions displayed; one real send not yet validated | Native session, since invalidated by the Chrome login | Historical; does not prove current authorization. |

Detailed narratives of the earlier follow-ups (identity-preserving model
edits, encrypted state storage, authentication recovery, newest-first
sorting, sticker and delivery status slots, group identity) are in the Git
history of this file before 2026-10-17 and in app-design.md.
