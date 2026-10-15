# Native messaging UX acceptance checklist

This checklist defines how the native app should behave during everyday use.
It is for the implementation session and subsequent UX verification. The
[Chromium UI map](chromium-ui-map.md) supplies reference evidence and feature
coverage; these are proposed native UX defaults, not additional observations of
Chrome. The [app design](app-design.md) supplies engineering constraints.

Implement the behavior as well as the screen. A populated screenshot does not
establish that navigation, scrolling, focus, recovery, or asynchronous updates
work correctly. Preserve the intent of each requirement when choosing toolkit
details. Record any deliberate behavior change here rather than silently
substituting an easier interaction.

## How to use the checklist

All boxes start unchecked. Check an item only after exercising its behavior,
and record the build, environment, scenario, and result in the verification log
at the end. Distinguish implemented from verified in work notes. Synthetic
fixtures should cover asynchronous states without accessing the account.

The core messaging gate comprises layout, navigation, chat list, history,
reading/copying, composer, sending, unread behavior, focus, and recovery. Added
surfaces such as media viewers, friends, group management, and settings must
meet their sections before being exposed as working features. A deferred
feature must not look actionable and then do nothing. Notification behavior
must be verified before enabling desktop notifications.

Current account constraint: Chrome owns the active login. Do not re-pair,
send messages, change account preferences, or emit read acknowledgements as
part of checklist verification without separate authorization. Mark account
checks blocked where necessary; continue with synthetic scenarios.

## Layout and independent scrolling

- [ ] L1. The navigation rail, browsing pane, conversation header, history, and composer have a clear visual hierarchy. Header and composer remain available while history scrolls.
- [ ] L2. The chat list, history, long draft, member list, and settings content scroll independently. Wheel or touchpad input over a pane never unexpectedly moves an adjacent pane. At a scroll boundary, input does not spill into another pane.
- [ ] L3. Each scrollable pane places its scrollbar at its trailing edge. Scrollbars have a usable pointer target, appear on hover/focus or while scrolling, and never cause text to rewrap when appearing. History and composer scrollbars are visually distinguishable.
- [ ] L4. Dragging a scrollbar keeps the thumb under the pointer. Incoming messages or changing media dimensions do not move the grabbed content unexpectedly; apply deferred repositioning after release while preserving the reading anchor.
- [ ] L5. Pane resizing, window resizing, display scaling, and font-size changes preserve the selected chat, draft, focused control, and visible reading anchor. Controls and text do not overlap at the supported minimum size.
- [ ] L6. Narrow layout has a deliberate list-to-conversation transition with a visible Back action. Returning to the list restores its position and selection. An open conversation remains identifiable and incoming unread state remains discoverable.
- [ ] L7. Text size settings affect message text and relevant surrounding labels without clipping. Long names, unbroken links, multiline previews, fallback fonts, and 100%, 150%, and 200% scaling remain usable.
- [x] L8. Empty space in the history does not stretch individual bubbles. Short messages use compact widths; long text wraps within the conversation pane. Timestamps and read state remain attached to their message.

## Navigation and conversation list

- [ ] N1. Friends, Chats, and Add friend change the browsing pane while preserving the open conversation and draft. In narrow layout, preserve that conversation when showing the browser instead.
- [ ] N2. Each section retains its query, category expansion, and scroll position during the session. Switching chat categories does not close a selected conversation outside the category.
- [ ] N3. Selection and keyboard focus have distinct visual treatments. The selected chat remains identifiable while focus is in search, the composer, or a popup.
- [ ] N4. Each chat row has stable areas for name, bounded preview, time, unread badge, and applicable mute/send-error indicators. Metadata does not obscure the name or preview. Full names remain available accessibly or on deliberate inspection.
- [ ] N5. New arrivals update preview and ordering without moving a row underneath an active click, drag, or context menu. When reordering completes, preserve the list's visible anchor; preserve the selected chat by identity.
- [ ] N6. Opening a chat leaves unread information visible until the relevant content is actually exposed. A new-message divider survives clearing the list badge long enough to orient the reader.
- [ ] N7. Loading additional conversations preserves the list position. Loading, end, failure, and retry states are distinguishable; retry does not duplicate rows.
- [ ] N8. Refresh retains usable cached content and selection. It shows local progress and a retriable error without replacing the entire screen with a loading state.

## Search

- [ ] Q1. Conversation-list search, friend search, and search within a conversation have explicit scope in their labels and empty states. If only loaded data is searched, disclose that limitation beside results.
- [ ] Q2. Search has a clear action and distinct loading, no-results, and failed states. A slow result for an old query never replaces results for the current query.
- [x] Q3. Filtering can hide the selected chat while keeping it open. Filtering never silently selects another chat or overwrites the draft.
- [ ] Q4. Up/Down moves through search results and Enter opens the focused result when IME composition is inactive. Escape first dismisses a search suggestion layer, then clears a nonempty query or exits search; it never closes the main window.
- [ ] Q5. Clearing list search restores the earlier list position. Search result updates preserve focus where the same result still exists.
- [ ] Q6. Conversation search reveals and highlights the selected match. Moving between matches does not lose the query; leaving search offers a return to the previous reading position.

## History entry and scroll dynamics

The reading anchor is the message currently being read plus its position within
the viewport. “At latest” means the bottom is within one normal text line of the
viewport bottom, no text selection or scrollbar drag is active, and the user
has not deliberately scrolled away. Programmatic layout changes alone must not
switch a reader into following new messages.

- [ ] H1. First opening a read chat shows the latest messages at the bottom. First opening an unread chat reveals its first unread message with a New messages divider and some preceding context where available.
- [ ] H2. Returning to a chat during the session restores its previous reading anchor. Preserve a separate unread boundary even when restoring an older reading position.
- [ ] H3. Incoming messages follow the bottom only while at latest. Reading older messages never triggers a jump, including when a receipt, reaction, image, or preview updates.
- [x] H4. While away from latest, show a compact Latest control above the composer. Add a count for new incoming messages below the viewport; simply having older loaded messages below does not count them as new.
- [ ] H5. Activating Latest goes to the bottom and resumes following. Sending a message also reveals the new outgoing item at latest. Typing, receiving a send confirmation, or receiving a read receipt does not independently move history.
- [ ] H6. Loading older history leaves existing visible content at the same position. Trigger loading before reaching the top; allow only one active request for that boundary and prevent repeated requests when already at the beginning.
- [ ] H7. Older-history loading, failure/retry, and beginning-of-history states appear at the top without covering messages. A failure leaves the current history readable and selectable.
- [ ] H8. Media loading, failed-media replacement, font changes, reaction changes, and message withdrawal preserve the reading anchor. If the anchored message disappears, retain its nearest surviving neighbor at the same position.
- [ ] H9. Switching chats during loading never flashes the previous chat's content or changes the new chat's scroll position. Returning to the first chat can show its completed content without losing its anchor.
- [ ] H10. Dragging to select text or dragging the scrollbar suspends following. After the interaction, resume following only if the user is at latest; do not snap to the bottom merely because new content arrived.
- [ ] H11. Date dividers distinguish calendar days. By default, consecutive messages from the same sender less than five minutes apart share one avatar/name and time label. A sender change, a gap of five minutes or more, a date boundary, or an intervening system event starts a new group. Unread dividers do not masquerade as messages.
- [ ] H12. Rapid arrival bursts do not repeatedly animate or bounce the viewport. Scroll animation can be interrupted by the user and respects reduced-motion preferences where available.

## Reading and message actions

- [ ] R1. Text supports pointer selection, keyboard selection, and Copy without entering an editor. Selection and links have clear focus/hover states; dragging across a link selects text rather than opening it on release.
- [ ] R2. Default copy contains the selected visible text with meaningful line breaks and inline emoji fallback text. It excludes timestamps, send status, and sender labels unless the user explicitly chooses a transcript-style action.
- [ ] R3. Native v1 supports selection within one message, including its text and inline emoji. Copy message copies its complete textual content; a reply's quoted content is included only when explicitly selected or requested. Cross-message drag selection is deferred and must not accidentally select unrelated labels.
- [ ] R4. Right-clicking selected text preserves the selection and offers Copy. Right-clicking an unselected message targets that message. Incoming updates and asset completion do not clear either selection or the active menu target.
- [ ] R5. Message actions are available through keyboard focus and context menus as well as hover. Revealing action buttons does not move the bubble, and menus remain inside the available screen area.
- [ ] R6. Reply previews reveal and briefly highlight their referenced message. Offer a return to the prior reading position. If unavailable, show a local explanation and keep the current position.
- [ ] R7. Link activation opens the intended URL only after deliberate activation. Full destinations are inspectable; displayed link text is not treated as proof of the destination.
- [ ] R8. Local deletion, withdrawal for participants, reporting, and leaving a group have distinct labels. Confirm consequential actions with their actual scope and retain context after cancellation or failure.
- [ ] R9. Unsupported, undecryptable, deleted, and unavailable messages have distinct readable placeholders. One bad message does not hide surrounding history or expose raw protocol errors.

## Composer and attachments

- [ ] C1. Enter inserts a newline or commits IME composition by default; explicit Send or Ctrl+Enter submits. The shortcut is clearly described and never consumes composition/candidate-selection keys.
- [ ] C2. The composer grows up to roughly one third of the conversation area's height, then scrolls internally. Its caret and final empty line remain visible; Send and attachment controls stay reachable and stable.
- [ ] C3. Sending is unavailable for whitespace-only text or a chat that cannot accept messages. The reason is visible or accessible. A lost connection does not lock editing or erase the draft.
- [ ] C4. Chat switching retains committed draft text and editor position. Clicking the already-selected chat never resets composition. Actual Fcitx preedit, candidate selection, cancellation, and focus changes pass the app-design acceptance checks.
- [ ] C5. Drafts survive normal app quit/restart in private account storage. An unsent attachment selection is restored where practical, or the user is told that files must be selected again. No draft silently moves to another chat/account.
- [ ] C6. After submitting, the editor is ready for the next message and retains focus. A late result cannot erase newer editing, move focus to another chat, or restore an old draft over a newer one.
- [ ] C7. Attachment selection opens a reviewable preview with destination, remove, cancel, and explicit Send. Cancel preserves the text draft. Selecting a file or dropping it never immediately transmits it.
- [ ] C8. Drag-over clearly identifies the destination conversation and drop target. Unsupported or oversized files have actionable local feedback; dropping outside a supported target does not navigate away or discard the draft.
- [ ] C9. Pasting text preserves line breaks. Pasting media follows the attachment preview flow. A long draft or attachment error cannot push essential controls offscreen.

## Outgoing message lifecycle

- [ ] S1. Submission immediately creates a pending outgoing item. Pending, server-confirmed, read, definite failure, and uncertain confirmation are visually and accessibly distinct, without moving the item's metadata area.
- [ ] S2. Confirmation updates the same outgoing item in place. Refresh, incoming events, reconnect, and reopening history do not duplicate it. Server confirmation never implies read.
- [ ] S3. The user can compose the next message while one is pending. If submission must wait, explain why beside Send rather than freezing the editor or unrelated chats.
- [ ] S4. A definite failure stays attached to the outgoing item with recovery actions. Restore/edit failed text deliberately; never overwrite another draft. Retry updates the original pending/failure presentation instead of leaving duplicate local bubbles.
- [ ] S5. An ambiguous result says that confirmation is unavailable and the message may have arrived. There is no automatic resend. A deliberate resend explains the duplicate risk and remains distinguishable from a safe retry after definite rejection.
- [ ] S6. Switching chats leaves pending/failure state attached to the correct conversation. Completion does not steal focus. Restart preserves unresolved send state so a possibly sent message is not silently presented as a fresh unsent draft.
- [ ] S7. Delayed receipts update quietly. Group reader counts appear only with supported semantics; a missing receipt is never styled as failure.

## Unread state and notifications

An incoming message is eligible to be marked read only when the conversation
window is active, its history is unobscured by an in-app modal, and the message
is exposed in the viewport. For a message taller than the viewport, exposing its
start is sufficient. Background windows, notifications, list previews, and
selecting a chat before its content appears do not establish reading. Honor
the protocol's actual watermark semantics; never claim per-message precision
that the server cannot represent.

| Situation | Expected attention behavior |
| --- | --- |
| App active, selected chat at latest | Update quietly; no desktop popup or sound. |
| App active, selected chat reading older history | Preserve position; show Latest/new-message count. No popup or sound for this selected chat. |
| App active, another chat receives a message | Update list and badge; show an in-app indication and play sound only if enabled. No desktop popup. |
| App inactive or minimized | Use desktop popup and sound according to preferences and mute state. |
| Muted conversation | Update unread state/list; suppress ordinary popup and sound. |
| Reconnect or initial history load | Reconcile unread state; suppress historical notification and sound bursts. |

- [ ] U1. Read eligibility follows the rule above. Messages below the viewport remain unread where protocol semantics allow it. Losing window activation stops read acknowledgements for new arrivals.
- [ ] U2. The unread divider continues to orient the user after read state updates; retain it until leaving the conversation rather than removing it underneath the reader.
- [ ] U3. App badge count means unread conversations, including muted ones. Per-chat badges mean unread messages when supported. Counts update consistently across reads on other devices and reconnect; unavailable counts do not appear as zero.
- [ ] U4. Notification activation opens the correct conversation and reveals the referenced message or relevant unread boundary. An existing draft survives. Dismissing a notification does not mark messages read.
- [ ] U5. Bursts from one conversation are grouped, with at most one sound per burst. Start with a two-second grouping window; notifications for distinct chats remain attributable. Test that repeated updates do not steal focus.
- [ ] U6. Mute suppresses mentions too by default. Any mention exception is an explicit preference. Group membership/system events obey their own enabled notification rules.
- [ ] U7. Preview privacy applies to titles and bodies: hiding message content must not accidentally disclose sender/group names if names are also disabled. Respect desktop/lock-screen privacy controls where supported.
- [ ] U8. Missing desktop permission has a persistent, recoverable settings state. Do not prompt repeatedly or claim notifications work when permission is denied. A settings test notification is explicitly initiated by the user.
- [ ] U9. Notifications already covered by a read update are withdrawn or updated where the desktop supports it. Own outgoing messages, receipts, reactions to old messages, and replayed events do not produce ordinary incoming-message alerts.

## Media and viewers

- [ ] M1. Photos preserve aspect ratio within bounded previews; transparent stickers have no oversized card or forced photo background. File, audio, video, contact, system, and text content each have appropriate presentation.
- [ ] M2. Reserve preview space when dimensions are known. Unknown dimensions use a bounded placeholder. Loading and failure are visible; retry does not collapse the row or jump the reader.
- [ ] M3. Opening media preserves conversation state. Escape closes the viewer and restores focus to its originating item; if it disappeared, restore a sensible nearby target.
- [ ] M4. Viewer previous/next scope is clear. Zoom, pan, fit, original size, and Save are discoverable. Viewer shortcuts never type into or submit the composer underneath.
- [ ] M5. Original-file download has progress, cancellation, failure/retry, and a clear saved result. Viewing a thumbnail does not falsely imply that the original is locally available.
- [ ] M6. Animation obeys autoplay preferences and pauses offscreen or while inactive. Reduced-motion users can read content without repeated attention-grabbing playback.

## Friends groups and auxiliary windows

- [ ] A1. Friend/category expansion and search preserve position. Opening a friend or group identifies whether it selects an existing conversation, opens a profile, or starts a new conversation; no ambiguous single click sends anything.
- [ ] A2. Starting a chat or creating a group shows selected recipients clearly, supports removal, and uses an explicit final action. Cancelling returns to the prior conversation/draft.
- [ ] A3. Group/member/profile windows show which conversation/person they belong to. Reopening an existing auxiliary window focuses it where appropriate instead of creating indistinguishable duplicates.
- [ ] A4. Member search and scrolling affect only that window. Permission-dependent actions have correct availability and explanations. Leaving, inviting, and editing show pending, completed, and failed states without misleading optimistic success.
- [ ] A5. Closing an auxiliary window does not close the conversation. If membership changes while it is open, update availability and explain why an action can no longer proceed.

## Settings focus and application lifecycle

- [ ] W1. Menus and dialogs place focus deliberately, support keyboard traversal, and return focus to their opener on dismissal. Escape closes only the topmost transient surface. Clicking outside dismisses ordinary menus, never submits a dialog.
- [ ] W2. Modal dialogs keep keyboard focus inside them. Background composer shortcuts, chat navigation, and read acknowledgements do not fire through an obscuring modal. Nonmodal windows remain independently usable.
- [ ] W3. Local settings apply immediately and show their effect. Account/profile edits use explicit Save/Cancel with pending, saved, and error states. Closing with unsaved account edits offers discard or return to editing.
- [ ] W4. Font, notification, shortcut, autoplay, and privacy settings have understandable labels and retained values. Unsupported controls are omitted or disabled with a reason; they never appear to save successfully.
- [ ] W5. Default main-window close quits. If a tray/background mode is later enabled, make close-to-background an explicit preference and explain that notifications continue. Quit always stops background activity; closing settings/viewers does not quit.
- [ ] W6. Restart restores the selected chat and drafts when authorized. Do not restore focus by unexpectedly opening a new modal or immediately marking unseen history read.
- [ ] W7. Reconnecting, offline, and expired authentication are distinct states. Keep readable history and editable drafts available where possible. Reconnect uses quiet persistent feedback; authentication failure provides a deliberate next action without automatically starting pairing.
- [ ] W8. Logout confirmation explains the fate of local drafts and cached content. Account switching never displays another account's cached messages, notifications, or drafts.
- [ ] W9. Errors appear beside the affected action with recovery where possible. A global status line is not the only record of a failed send, failed page load, or disabled notifications. Technical diagnostics stay out of ordinary message content.
- [ ] W10. Every actionable control has a usable keyboard path, visible focus, and an accessible label/state. Pending/read/error states and selection do not depend on color alone. New arrivals do not steal focus or repeatedly announce the entire history to assistive technology.

## Required interaction scenarios

Run each scenario with representative Japanese text, emoji, long names, and
mixed content. Use real desktop input for pointer selection, keyboard focus,
clipboard, scrollbar dragging, and IME; setting widget values alone does not
verify these behaviors.

| Scenario | Required outcome |
| --- | --- |
| Reading halfway through history; three arrivals; an image above finishes | Visible text stays in place, selection survives, new-message control shows three, and unseen arrivals remain unread. |
| At latest; receive a burst with mixed text and media | Latest content remains visible, media preserves geometry, and the app does not repeatedly bounce or steal focus. |
| Drag scrollbar while an older page and new messages arrive | Thumb stays under the pointer; release leaves the intended reading area visible. |
| Load older history, fail once, retry, then reach the beginning | No anchor jump or duplicate rows; failure is local; end state stops further loading. |
| Switch chat during load/send, type a newer draft, then receive old results | Correct content/draft stays visible; send status stays with its chat; focus does not move. |
| Submit, lose confirmation, restart, reconnect with a matching server message | Uncertain state survives restart and reconciles to one confirmed item without resending. |
| Select multiline text with inline emoji; receive reactions and media | Selection survives; clipboard has meaningful text/line breaks and no unrelated metadata. |
| Search hides selected chat; results reorder; clear search | Open conversation/draft stay intact; focused result stays predictable; original list position returns. |
| Jump to an old reply/search match, then return | Correct target is highlighted; original reading anchor is recoverable. |
| Receive while active, inactive, minimized, muted, and reconnecting | Attention behavior matches the notification table; no historical alert burst or focus theft. |
| Activate a notification while composing in another chat | Destination opens at relevant content; prior draft survives; dismissal alone never marks read. |
| Resize at large text/HiDPI with a long draft and media | Independent scrolling works; caret, Send, metadata, and selected content remain usable. |
| Compose Japanese; switch focus, cancel candidates, select the same chat | No accidental send, duplicated commit, stuck preedit, or lost committed draft. |
| Open menu, viewer, member window, and settings; dismiss in turn | Only intended surfaces close; focus returns correctly; underlying shortcuts do not fire. |

## Implementation checkpoint — 2026-10-15

This is still an incomplete acceptance gate. A checked box refers to exercised
behavior; partial verification below does not complete every requirement with
that ID. Most boxes deliberately remain unchecked.

Implemented in this pass: narrow list/conversation navigation; explicit loaded
search scope and Clear; distinct row focus; compact selectable text with copy
menus; per-chat drafts and reading/editor scroll state; latest/follow behavior;
stable thumbnail space and deferred updates during a history thumb drag;
identity-preserving message model edits; local list/history/media errors;
whitespace/size validation; durable pending and uncertain outgoing items;
known-server-ID reconciliation; append-to-draft recovery; encrypted private
restart storage and immediate retained text-size controls. Metadata from a
completed send never implies read. Draft storage stays separate from expiring
caches. Storage failures prevent submission and remain visible.

Remaining implementation work includes virtualized older-history paging;
conversation unread metadata; live synchronization and reconnect;
unread boundaries and viewport-gated acknowledgements; date/sender grouping;
conversation search and reply jumps; inline LINE emoji spans and link actions;
definite-rejection classification and safe retry; reliable reconciliation of
an ambiguous send without a server ID; full editor cursor/selection restoration;
and offline account startup. Reading-view caches currently retain eight chats;
font/resize anchor compensation and scrollbar dragging still need physical
input verification. Composer IME acceptance remains a release gate.

Friends/Add friend, attachments, group/member/profile management, video/audio viewers,
account editing/logout, and notification settings are unavailable rather than
exposed as working controls. Desktop notifications/read acknowledgements stay
disabled. The browsing pane labels Friends/Add friend as unavailable; history
labels its recent-only limit. These are disclosed gaps, not changes to the
requirements above. New chat arrivals keep the loaded list order stable in this
version at this earlier checkpoint; the presentation follow-up below replaces
that behavior with latest-activity sorting.

## Verification log

Record requirement IDs and scenario names, keeping account contents and private
screenshots out of Git. Account-dependent checks remain unchecked until they
can be exercised with authorization.

| Build or commit | Requirement or scenario | Environment | Result and remaining issue |
| --- | --- | --- | --- |
| 2026-10-15 working tree, release | L8; Q3; H4 | Synthetic headless 980×720, 740×520, 360×520; text 15/24; native Wayland Slint 1.18.1, software, scale 1.5 | Passed compact/wrapping bubble layout, filtering without replacing conversation/draft, and Latest with exactly three new arrivals while older loaded messages alone have no count. |
| Same build | H2–H5, H8 partial; R1–R4 partial; S4–S6 partial; C6 partial | `tools/check-ui-mcp.py --scenarios`, offline Wayland preview, scale 1.5 | Simulated scroll to older text, three delayed arrivals, fixed-space media completion: visible message stayed at the same Y. Keyboard-selected multiline Japanese/emoji survived updates; clipboard round trip matched text exactly without metadata. Latest resumed following and a subsequent burst stayed visible. Uncertain outgoing result retained a newer draft; recovery appended text. Repeated run with existing outbox also passed. This is toolkit event simulation, not physical pointer/IME acceptance; broader requirements stay unchecked. |
| Same build | Q3; N7/N8 partial; H9 partial; S2/S3/S5/S6 partial; C3/C5/C6 partial | `cargo test --release --offline --workspace`; synthetic sessions, headless controller, in-memory command queue | 13 passed, 1 existing WASM-dependent test ignored. Exercised late response isolation, cached views after failure, known-ID confirmation deduplication with stable local row/model, whitespace rejection, durable pending-before-submit, newer draft preservation, and no automatic ambiguous resend. Encrypted restart, account isolation, permissions, corruption rejection, and text-size persistence passed. No account worker was started. |
| Same build | W1 partial | Offline Wayland preview, MCP right-click then Escape | Copy selection/Copy message menu appeared; Escape dismissed it and the main window survived. Full keyboard traversal and focus-return checks remain pending. |
| Same build | Build checks | Linux, pinned Slint 1.18.1 | Release workspace tests, formatting, and Clippy with warnings denied passed. Headless snapshots and synthetic screenshots kept under `/tmp`, outside Git. |
| Same build | Account-dependent requirements; physical input/IME scenarios | Chrome owns active login | Blocked for account verification without separate authorization. No native account startup, pairing, send, preference mutation, or read acknowledgement was used. Physical pointer drag, Fcitx, and 100%/150%/200% desktop checks remain unverified. |


### Authentication recovery follow-up

Failed account startup now keeps its command receiver alive and exposes a
persistent Retry/Sign in again banner, including in narrow conversation view.
Retry reconnects with the saved session. Sign in again opens an explanation;
only Start sign-in begins QR pairing. Native QR/PIN display, cancellation,
retriable failure, and successful reconnection use the existing pairing protocol.
Successful pairing preserves the previous session file and its separately keyed
drafts, clears the old account's visible data, and opens the new account storage.
Native QR/PIN data stays in memory. Phone approval may replace Chrome's login.

Synthetic tests verified failed-startup retry, explicit command handoff,
cancellation without pairing, QR/PIN presentation and clearing, draft retention
on dismissal, session backup permissions, and removal of old UI/account state on
successful handoff. Offline desktop verification used
`tools/check-ui-mcp.py --scenarios --auth-recovery`. The real phone/QR/PIN exchange
was deliberately not exercised; W7 remains unchecked pending end-to-end account
verification and distinct authentication/transport error classification.

Follow-up build checks: 15 workspace tests passed, one existing WASM-dependent
test ignored; formatting and Clippy with warnings denied passed. Synthetic
Wayland recovery and messaging scenarios passed at scale 1.5. No real account
connection or pairing was performed during these checks.


### Conversation and media presentation follow-up

Conversation rows now sort newest-first using their latest message timestamp;
refresh, paging, and local submission retain the selected chat by identity.
Pointer-held list updates wait until the click/drag completes. Message histories
sort oldest-first, including cached pages and local pending items; ties use the
numeric message ID order. List previews normalize line breaks and clip/elide
inside fixed row bounds. Contact photos are center-cropped and circle-masked,
with explicit image dimensions inside their 40 px frame. The software renderer
cannot supply rounded image clipping by itself.

Read-only message selection keeps native Copy/keyboard selection and hides the
insertion caret. Photo and timestamp geometry use separate bounded layout
areas. Clicking a sent or received photo opens an original-file viewer with
Fit, zoom, 1:1, scrolling, retry, and Escape/Close. A late result cannot replace
a different/dismissed viewer. The original image stays outside the thumbnail
cache and is released on close. Text-size controls moved to Settings; the main
screen no longer exposes size +/− buttons.

Offline tests cover newest/oldest ordering, same-time message IDs, pending-item
chronology, deferred list sorting, draft/selection identity, source-photo URL
construction, full-resolution decoding versus thumbnail dimensions, and avatar
masking. `tools/check-ui-mcp.py --scenarios --layout-photos --auth-recovery`
checks actual widget bounds, timestamp/image separation, photo activation,
original-size/Fit behavior, Escape, and retained drafts. Headless photo fixtures
cover 360×520 at 24 px text. Native verification uses software rendering on
Wayland at scale 1.5. Real original-photo fetching, physical input/IME, and the
remaining complete media-viewer acceptance requirements stay unverified.

Presentation follow-up checks: 16 workspace tests passed, one existing
WASM-dependent test ignored; formatting and Clippy with warnings denied passed.
The combined offline messaging, photo/layout, and auth-recovery desktop checks
passed. No live account connection, original-file fetch, send, or pairing was
used for verification.

### Sticker, delivery status, and shortcut follow-up

Sticker fallback text appears inside the reserved image frame while loading;
the loaded image replaces it without changing message or timestamp geometry.
Timestamp and delivery state now occupy separate areas. Outgoing messages reserve
one line for Sending…/Sent/Unconfirmed/Not sent, including historical messages.
Uncertain and failed submissions retain their explanation and draft-recovery
action separately. Confirmed means server confirmation, never a read receipt.
Reconciliation preserves the local row identity and submitted timestamp, so a
history refresh does not replace the date or remove the delivery label.

Ctrl+Enter invokes the same guarded action as Send without inserting a newline.
Plain Enter remains native multiline/IME input. Repeated keypresses, disabled
submission, active native preedit, and obscuring settings/viewer/sign-in screens
cannot trigger the shortcut. Native preedit events are observed and propagated
without replacing the standard TextEdit.

Synthetic controller verification exercises enabled and disabled Ctrl+Enter,
plain Enter, the composition-state guard, and timestamp/status retention through
confirmation and server reconciliation. The offline Wayland MCP checks verify
disabled Ctrl+Enter retains the draft and a delayed sticker image removes the
fallback without moving its timestamp. Combined messaging, photo/layout, and
authentication recovery scenarios passed at scale 1.5. Workspace tests passed
(16 passed, one existing WASM-dependent test ignored); formatting and Clippy
with warnings denied passed. Physical Fcitx composition/candidate selection
remains unverified, so C1/C4 remain unchecked. No live account or send was used.

### Group identity and compact metadata follow-up

Group messages retain sender IDs, resolved contact names, and sender-avatar
assets through protocol decoding, encrypted history caching, the account worker,
and the UI. Contact lookup is bounded to distinct senders on the current page;
failure keeps readable messages and cached identity where available, with a
distinct fallback for unknown senders. Legacy cache pages without sender IDs
attempt refresh and remain readable if it fails. Avatars use the existing
circle-masking loader and fixed 32 px slots, with initials until loaded.

The default grouping applies in direct and group conversations: adjacent
messages from the same identity with a gap strictly below five minutes share
one time label. Group-chat avatar/name appears once per group. Sender changes,
five-minute gaps, and local calendar-day changes break the group. Names alone
never determine identity; unknown group senders do not collapse into one person.
Calendar dates appear once per day. Short HH:mm times and compact delivery
marks sit beside bubbles in fixed slots, replacing the earlier separate date
and status rows. Delivery marks retain accessible labels distinguishing Sending,
server confirmation, and uncertainty; confirmation never implies read. Failed
and uncertain messages still carry their full explanation and recovery action.

Offline tests cover the strict five-minute boundary, sender identity despite
matching names, day changes, outgoing grouping, unknown senders, sender metadata
cache round trips and legacy compatibility, and delayed sender-avatar updates
without replacing the message model. The combined Wayland MCP scenarios verify
group names/avatars, one time per group, date dividers, metadata beside photo
and text bubbles, stable sticker completion, clipboard/scroll behavior, photo
viewing, drafts, and auth recovery at scale 1.5. Real group-contact lookup and
physical-input acceptance remain unverified; H11 stays unchecked pending full
system-event and unread-divider handling. No live account access was used.

Group follow-up checks: 17 workspace tests passed, one existing WASM-dependent
test ignored; formatting and Clippy with warnings denied passed. Headless
`--demo-groups` snapshots at 980×720/15 px and 360×520/24 px confirm bounded
sender avatars, compact grouped text, and metadata outside bubbles. Snapshots
remain under `/tmp`; the final release has the debug server disabled.
