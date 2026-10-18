# Native client UX model

Status: normative. Written 2026-10-17 after auditing the Slint UI, controller,
worker, tooling, and desktop integration as of commit `fc4c112`.

This document defines how the native LINE client behaves as a desktop
application. [ux-acceptance.md](ux-acceptance.md) turns it into checkable
requirements and records what is verified. [ux-sessions.md](ux-sessions.md)
is the register of open problems found by audit, ordered for fixing.
[app-design.md](app-design.md) owns engineering constraints (toolkit, threads,
storage, crypto). [chromium-ui-map.md](chromium-ui-map.md) is reference
evidence about LINE Chrome, not a specification for this app.

When a session has to choose between this model and an easier toolkit path,
it follows the model or records the deviation in the acceptance document.
A screenshot never proves a behaviour in this model.

## 1. Product stance

The app is a quiet, trustworthy desktop messenger for one LINE account on a
Linux desktop (KDE Plasma on Wayland first, X11 and other desktops tolerated).

Priorities, in order. A lower priority never overrides a higher one.

1. **Never lose what the user typed.** Drafts, caret, pending messages, and
   recovery text survive navigation, errors, restarts, and account recovery.
2. **Never lie about delivery, reading, or freshness.** Pending, confirmed,
   read, rejected, and uncertain are distinct. Stale content is marked stale.
   A count that is unknown is not shown as zero.
3. **Never move what the user is reading.** Asynchronous updates preserve the
   reading anchor; only the user's own actions scroll the view.
4. **Interrupt only when the user is not already looking.** Attention
   escalates from in-app indicator to badge to desktop notification to sound
   based on window state, mute, and preferences.
5. **Be a good desktop citizen.** One instance, correct app identity, system
   theme and scale, native notifications and badges, XDG paths, IME first.
6. **Say less.** Controls state what they do. Limitations are disclosed once,
   in the place where the user hits them, not as permanent captions.

## 2. Vocabulary

| Term | Meaning |
| --- | --- |
| Surface | A region with its own focus, scroll, and state: rail, browser pane, conversation pane, detail panel, dialog, menu, viewer. |
| Layer | Stacking class of a surface: base, panel, dialog, sheet, menu, transient. Defined in §3.2. |
| Wide / narrow | Window width ≥ 740 logical px shows browser and conversation side by side; below that they stack and one is shown at a time. |
| Reading anchor | The topmost fully or partially visible message plus its offset from the viewport top. |
| At latest | The history bottom is within one text line of the viewport bottom, no text selection or scrollbar drag is active, and the user has not scrolled away since the last user-initiated move to the bottom. |
| Following | While at latest, new messages keep the view at the bottom. Following is a consequence of being at latest, never a separate toggle the app flips on its own. |
| Exposed | A message whose start is inside the viewport of an active, unobscured conversation (see §7). |
| Unread boundary | The position before the oldest message not yet exposed in this conversation, as known locally. |
| Active window | The main window has keyboard focus according to the compositor. Occluded or minimised windows are inactive. Unknown is treated as inactive. |
| Attention event | Something that may need the user: incoming message, mention, group event, send failure, authentication expiry, connection change. |
| Definite failure | The server or transport rejected the request and the message certainly did not arrive. |
| Uncertain result | No confirmation was received; the message may have arrived. |
| Loaded scope | Data already in memory. Searches over loaded scope must say so in the empty state, not as a permanent caption. |

## 3. Surface model

### 3.1 Shell

```
┌──┬───────────────┬────────────────────────────────┬──────────┐
│R │ Browser pane  │ Conversation pane              │ Detail   │
│a │ (Chats /      │  header                        │ panel    │
│i │  Friends)     │  history (scrolls)             │ (opt.)   │
│l │  search       │  new-message / latest control  │          │
│  │  list (scroll)│  composer (grows, scrolls)     │          │
│  │  footer       │  caption + Send                │          │
└──┴───────────────┴────────────────────────────────┴──────────┘
```

- **Rail**: fixed, icon-first destinations Chats, Friends, and (when
  implemented) Add friend at the top; Settings at the bottom, including
  account information in About.
  The current destination is *selected*, never *disabled*. Unavailable
  destinations are omitted, not shown as apology pages.
- **Browser pane**: one destination at a time. Each destination keeps its own
  query, filters, expansion, selection focus, and scroll offset for the
  session. The pane never owns the conversation; switching destinations
  leaves the open conversation and draft untouched.
- **Conversation pane**: header, history, latest control, composer. In wide
  layout it is always visible. In narrow layout it replaces the browser pane
  and has a Back control in the header.
- **Detail panel**: optional right-side, non-modal surface for a profile,
  group members, or conversation search results. It never hides the history.
  At 740–1059 px a profile becomes a sheet over the conversation, preserving
  the browser and rail. Below 740 px it covers the whole shell. The 1060 px
  breakpoint adds a 320 px panel to the 740 px browser/conversation boundary;
  the conversation allowance derived from that boundary is 398 px
  (740 − 56 rail − 286 browser). Connection feedback stays in the uncovered
  conversation/list; profiles keep their position.

### 3.2 Layers and when to use each

| Layer | Use for | Modal | Dismiss | Example |
| --- | --- | --- | --- | --- |
| Base | Shell | no | — | Rail, panes |
| Panel | Secondary content that complements the conversation | no | Close button, Escape when focused | Profile, members, in-chat search |
| Dialog | A decision with consequences or a short form | yes, to the main window | Explicit buttons; Escape = cancel | Leave group, logout, send file confirmation |
| Sheet | Takeover of the conversation or whole shell | to the covered surface | Back/Escape | Profile when panes cannot fit; sign-in, media viewer, panels/dialogs in narrow layout |
| Menu | Actions on one target | transient | Click outside, Escape, activation | Message context menu, more actions |
| Transient | Brief status not needing action | no | Timeout or next state | "Copied", "Reconnected" |

Rules:

- Settings is a dialog in wide layout and a sheet in narrow layout. It never
  blanks the conversation in wide layout.
- A sheet may be used in wide layout for sign-in, the media viewer, and a
  profile over the conversation when the conversation and panel cannot fit.
- At 740–1059 px the profile sheet is modal only to the covered conversation:
  its composer, history, shortcuts and read acknowledgements are blocked, and
  covered content is hidden from accessibility. The browser and rail remain
  available to pointer, focus and keyboard navigation. Browser shortcuts work
  from the sheet too; destination/chat navigation closes it according to the
  profile binding rules. Escape from anywhere closes the sheet before clearing
  a browser search. Below 740 px the sheet is modal to the whole shell and hides
  it from accessibility and also blocks browser shortcuts.
- Escape closes exactly the topmost layer above base. At base, Escape in a
  search field clears the query; elsewhere it does nothing. Escape never
  closes the main window and never cancels an in-flight pairing without a
  confirmation step.
- Opening a layer records the opener; closing restores focus to the opener
  if it still exists, else to the nearest surviving control.
- Modal layers block keyboard shortcuts, read acknowledgements, and pointer
  input beneath them. Panels do not.

### 3.3 Narrow layout

Below 740 px the browser pane and conversation pane alternate. Back returns
to the browser pane at its previous offset and selection. Selecting a
conversation shows the conversation pane and focuses the composer. The
rail collapses to a bottom or top bar with the same destinations. Unread
state must be reachable from the conversation pane (badge on Back or rail).

### 3.4 Profile, member and group surfaces (U0, approved)

Reference: LINE Chrome's profile and group popups (local screenshots, summary in
[protocol-traces.md](protocol-traces.md#reference-screenshots-u0)). LINE opens
them in a separate window; here they live in the detail panel (§3.1), so the
conversation stays visible at ≥1060 px.

**One stack.** Profile, group info and members are pages of one panel stack,
for example group info → Members → member profile. The header has a back
arrow when the stack is deeper than one page, a title, and × to close. Escape
and Back pop one page, and × closes the whole stack. The same stack is the
sheet at 740–1059 px and the full-window sheet below 740 px. The binding rules
of S2 apply to the stack as a whole.

**Entry points.**

| From | Opens |
| --- | --- |
| 1:1 header title or avatar | that contact's profile |
| Group header title | group info |
| Group message sender avatar or name | member profile |
| Members row | member profile, pushed onto the stack |
| Friends row, Profile action | friend profile; the group row opens that group's chat with group info |
| Settings account row | own profile |

**Profile page** (top to bottom; parts with no data collapse, never leaving
gaps):

- A 96 px cover band: the cover image when P6 provides it, otherwise a tint of
  the accent colour.
- A 96 px circular avatar centred on the band's lower edge. Activating it opens
  the viewer. Without a picture, an initial on a tinted circle, never a bar.
- Name at heading size, through the rich-text path so emoji render, with at
  most two lines and an ellipsis. When the user renamed the contact, the
  original name follows in secondary text.
- Status message in secondary text, at most four lines, with no scroll area of
  its own.
- An action row: 40 px circular icon buttons, each with a small label below,
  centred, depending on the relation:

| Relation | Actions | Notes |
| --- | --- | --- |
| Friend | Chat, Rename | Rename opens a one-field dialog (`updateContactSetting`). Favourite is shown read-only (star after the name) until its write is traced. |
| Non-friend member | Add friend | `addFriendByMid`. No Chat, as in LINE. A fraud notice from `getTargetProfileNotice` sits above the actions as an inline notice. Block and Report are absent until traced. |
| Unknown member (no contact data) | none | Name "Unknown member", initial avatar, no status. |
| Own | none | Read-only: name, status and avatar. Editing is out of scope. |
| Official account | Chat when a chat exists | No Add friend until traced for OAs. |

- A More (⋯) header button with Copy name.

**Group info page:**

- Avatar (96 px, no cover band) and name.
- "N members" in secondary text.
- Rows, each a full-width list row:
  - Members (N) ›, which pushes the members page;
  - Notifications, a switch (`updateChat` `notificationDisabled`);
  - Leave group, in the danger colour, behind a confirmation dialog
    (`deleteSelfFromChat`). After leaving, the chat stays read-only in the list
    until the server removes it.
- Invite, Photos, Links and Files are absent until traced. A room (unnamed
  multi-user chat) has no Notifications row and no Leave row and is titled
  with its member names.

**Members page:**

- Search field.
- Rows: 40 px avatar, name, one-line secondary status, and a "Friend" badge.
  "You" comes first.
- Activating a row pushes that member's profile. For a friend, a Chat icon
  appears on the row on hover or focus.
- Invitees follow under "Invited (N)", with no actions.

**States.** Pages render from the store at once. There are no spinners and no
"loading" text; fields that are still missing collapse. Failed fetches show
nothing, and the engine retries them (no refresh controls). A running action
disables its button and shows a progress label ("Adding…"). A failed action
shows one line of danger text under the action row and re-enables the button.
A successful action updates the page in place: Add friend turns the page into
the friend page.

**Focus.** Header controls (Back, More, ×), then the avatar, then the actions
from left to right, then rows in order. Opening a page focuses its first
control after Back. Popping a page restores focus to the row or control that
pushed it. §8.2's panel and sheet cycles apply to the stack.

**Sizes.** Panel 320 px. Content padding 16 px, gaps 8 and 12 px. The action
buttons share the width equally, with a maximum of 72 px each.

```
 wide (panel)                      narrow (sheet)
┌──────────────────────┐          ┌──────────────────────┐
│ ←  Profile     ⋯  ×  │          │ ←  Profile     ⋯  ×  │
│▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│          │▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│
│▓▓▓▓▓▓▓▓( ◯ )▓▓▓▓▓▓▓▓▓│          │▓▓▓▓▓▓▓▓( ◯ )▓▓▓▓▓▓▓▓▓│
│        Aoi ☆         │          │        Aoi ☆         │
│     (Aoi Tanaka)     │          │   status message…    │
│   status message…    │          │   (◯)Chat (◯)Rename  │
│  (◯)Chat (◯)Rename   │          │                      │
└──────────────────────┘          └──────────────────────┘
```

## 4. State model

### 4.1 Per conversation (session memory, bounded to recent N)

draft text, caret and selection, composer scroll, reading anchor, at-latest
flag, unread boundary, new-arrival count while away from latest, loaded
message window and older-history cursor, in-chat search query and current
match, pending/uncertain outgoing items (also persisted).

### 4.2 Per destination (session memory)

query, category filters, expansion state, focused result identity, scroll
offset, saved offset before a search began.

### 4.3 Persistent (encrypted private state, see app-design)

drafts, outbox with status and timestamps, selected conversation, text size,
notification and composer preferences, mute overrides if the protocol does
not supply them, last known unread counts for restart display (marked stale
until synced).

### 4.4 What survives what

| Event | Draft + caret | Reading anchor | Selection of chat | Destination state | Overlays |
| --- | --- | --- | --- | --- | --- |
| Switch chat | kept per chat | kept per chat | changes | kept | closed if chat-bound |
| Switch destination | kept | kept | kept | per destination | kept |
| Open/close panel, dialog | kept | kept | kept | kept | — |
| Resize, scale, text size | kept | kept (anchor re-applied) | kept | kept | kept |
| Refresh, incoming data | kept | kept | kept by identity | kept | kept |
| Lose/regain window focus | kept | kept | kept | kept | kept |
| Restart | draft text kept; caret best effort | not kept (open at unread boundary or latest) | kept | reset | closed |
| Account change | isolated per account | reset | reset | reset | closed |

## 5. Reading model

The history is a list ordered oldest to newest with date dividers, sender
groups, system items, and an unread divider. These invariants hold for every
mutation (append, prepend, in-place update, removal, media size change,
font change, resize):

1. If a drag, selection, or context menu is active, model changes are
   deferred until it ends.
2. If at latest, the view stays at the bottom after the change.
3. Otherwise the reading anchor keeps its screen position. If the anchored
   message disappears, its nearest surviving neighbour takes its place.
4. The view scrolls programmatically only on: first open of a chat, the
   user activating Latest, the user activating a jump (reply, search match,
   notification), the user sending a message. Nothing else scrolls.

First open shows the unread boundary with some context above it, or the
bottom if nothing is unread. Reopening within the session restores the
previous anchor. New-arrival count while away from latest counts incoming
messages appended below the viewport since the user left latest; older
loaded messages below the viewport are not "new".

Older history loads when the viewport approaches the top, one request per
boundary at a time, with loading/end/error presented inside the history as
items that do not shift content. Rows must be virtualised or windowed before
the loaded window exceeds a few hundred messages.

## 6. Outgoing message lifecycle

```
Composing ──submit──▶ Pending ──confirmed──▶ Confirmed ──receipt──▶ Read
                        │                       ▲
                        ├──definite rejection──▶ Rejected ──retry (safe)──┘
                        └──no answer──────────▶ Uncertain ──user resend (duplicate warning)──▶ Pending
```

- Submit creates a durable Pending item before any network call and clears
  the composer. The item keeps its local identity through confirmation and
  reconciliation with the server ID. Refresh, reconnect, and restart never
  duplicate it.
- Confirmed means the server accepted it. It never implies Read. Read comes
  from a receipt or watermark with the protocol's actual precision; group
  reader counts appear only when the protocol supplies them.
- Rejected offers Retry (safe) and Edit (moves text to the composer without
  overwriting a newer draft). Uncertain offers Resend with a duplicate
  warning and Edit. There is no automatic resend of an Uncertain item.
- States are shown by a compact mark beside the time with an accessible
  label, in a fixed slot that never changes the bubble geometry. Rejected
  and Uncertain additionally show a one-line explanation with actions under
  the bubble.
- Restart turns Pending into Uncertain, never into a draft.
- A transport failure for one send never changes the connection state of the
  whole app; connection state comes from the sync channel (§7.4).

## 7. Unread, read, and attention

### 7.1 Sources

Unread counts come from the server chat list and live events. The inspected
Chrome mute flag is private user storage (`messageBox[id].isMute`), not a
server chat field; native mute remains unknown until a native private local
override is available, and must not be presented as a known server preference.
The protocol's `sendChatChecked(reqSeq, chat, messageId, 0)` advances a
per-chat last-seen message-ID watermark; operation 40 carries chat in param1
and that watermark in param2. It clears that chat's unread count. Receipt
operation 55 instead advances one member's read-range endpoint (chat/member/end
in param1/2/3); fetched ranges preserve start/end IDs and the open-end sentinel
`1`, so group counts must be calculated for each message from distinct member
ranges, never from a single receipt endpoint. Source coverage is start-exclusive,
end-inclusive (`startMessageId < messageId <= endMessageId`). The initial end
sentinel `1` covers no messages; it is preserved as an open/unadvanced range,
never interpreted as infinity or a Read indication. Message IDs compare as positive
decimal integers without converting them to floating point.

Local exposure updates the local boundary immediately and sends the read
marker when eligible. Reads on other devices arrive as events and clear
local state. Counts that are unknown show as a dot, not as a number.

### 7.2 Read eligibility

A message becomes eligible to be marked read only when all hold: the main
window is active; the conversation pane shows this conversation and no modal
layer covers it; the message start is exposed in the viewport; the user did
not merely select the chat while its content was still loading. Losing
activation stops marking. A read marker is sent at most once per boundary
advance and coalesced within one second.

### 7.3 Unread divider

The divider appears on first open at the boundary and stays until the user
leaves the conversation, even after the server boundary advances.

### 7.4 Attention matrix

Event classes: **message** (incoming in a conversation), **mention** (message
addressing the user in a group), **group event** (invite, join, leave),
**send failure**, **account** (authentication expired, unavailable session, offline).
Routine reconnects are quiet header state and never an account alert.

| App state | message | mention | group event | send failure | account |
| --- | --- | --- | --- | --- | --- |
| Active, same chat, at latest | update quietly | update quietly | inline item | inline on item | recovery strip |
| Active, same chat, reading older | latest control + count | as message, mark mention | inline item | inline on item | recovery strip |
| Active, other chat | list badge, rail badge, app badge | same + in-app indicator | list badge | list marker + app badge | recovery strip |
| Inactive (unfocused, occluded, minimised) | desktop notification, app badge, attention hint, sound | same, bypasses "mentions only" filter | notification if enabled | notification | notification once |
| Muted conversation | badge only | badge only unless mention exception on | per pref | as above | — |
| Do not disturb (desktop) | badge only; the daemon suppresses popups | same | same | same | same |
| Reconnect or initial sync | reconcile counts; no notification burst, no sound | same | same | — | transient "Reconnected" |

Rules:

- One notification per conversation, replaced in place as more messages
  arrive (replaces-id). A burst produces at most one sound per conversation
  per two seconds.
- Activating a notification opens that conversation at its unread boundary,
  focuses the window, and preserves any draft elsewhere. Dismissing a
  notification marks nothing read.
- Notifications are withdrawn when their conversation is exposed, when the
  messages are read elsewhere, or on logout.
- Reconnecting is a quiet inline “Connecting…” header state. Recovery shows
  “Reconnected” briefly below the headers. Neither produces desktop notification
  or sound. Offline and AuthExpired show an overlaid one-line recovery strip
  with Retry. An unavailable session at startup/connect failure also offers
  Sign in again; so does AuthExpired, including after a failed Retry. Network
  failure alone after opening a usable session offers Retry only.
- The strip sits directly below the headers over history, or over the list in
  narrow browser layout. The scroll range gains a top inset equal to the strip
  height. Offset compensation preserves the visible content when feedback appears
  or disappears; scrolling to the top (including Home) reveals the first item
  below the strip. Offsets stay in range; any residual visual inset after hiding
  feedback is consumed by scrolling toward later content, without reversing a
  scroll gesture. Home removes it explicitly. Exposure and navigation use the
  remaining visible viewport height. Status text elides at the chosen text size;
  its accessible label retains the full description.
  Rail, header, composer and profile controls are never covered.
  Profiles keep their position and can cover connection feedback as a modal
  sheet. Retry shows “Retrying…” with actions disabled while it is pending.
- Own outgoing messages, receipts, reactions to old messages, and replayed
  history never produce incoming-message alerts.
- Preview privacy levels: name and text; name only; "New message" only. The
  avatar is included only at the first level.
- The app badge counts unread conversations including muted ones; a
  separate pending-failure marker is not folded into that number.

## 8. Input model

### 8.1 Keyboard

| Shortcut | Action | Scope |
| --- | --- | --- |
| Ctrl+Enter | Send | composer, outside IME composition |
| Enter | Newline (default); Send when the preference "Enter sends" is on and composition is inactive | composer |
| Shift+Enter | Newline always | composer |
| Escape | Close topmost layer; clear search; otherwise nothing | global |
| Ctrl+F | Focus browser search, or in-chat search when the conversation has focus | global |
| Ctrl+1 / Ctrl+2 / Ctrl+3 | Friends / Chats / Add friend (matches LINE Chrome) | global, not during composition |
| Alt+Up / Alt+Down | Previous / next conversation in the visible list | global |
| Alt+Left or Back key | Back in narrow layout | conversation pane |
| Up / Down / Home / End / PageUp / PageDown | Scroll history | history has focus |
| Tab / Shift+Tab | Move between controls, not between messages | global |
| Ctrl+C | Copy selection | focused message or field |
| F6 | Focus connection recovery actions | when the connection strip is visible |
| Ctrl+Q | Quit | global |

IME composition always wins: while a preedit is active no shortcut fires
and Enter commits the composition. Shortcuts are listed in Settings and are
not configurable in v1.

### 8.2 Focus

- Focus order: rail → browser search → list (one Tab stop; arrows move
  within) → conversation header actions → visible connection recovery actions →
  history (one Tab stop) → Latest → Return to message (when visible) →
  composer → Stickers (when enabled) → Send.
- Chats filter chips are one Tab stop after browser search (and Clear when present), then New chat, then the list; Left/Right moves between chips and Space/Enter activates.
- Friends category headers are rows in the friends list; arrow navigation reaches them and Enter/Space toggles collapse.
- F6 enters the visible recovery strip. Without a layer, F6 or Tab after its
  last action moves to history, or the browser list in narrow browser layout. Shift+Tab before
  its first action returns to the conversation header/browser search. In narrow
  browser layout the strip belongs between browser search and the list.
- Friends uses the same recovery-action order as Chats. A nonempty search
  adds Clear immediately after the field in either browser and in members.
- A side panel follows Send (or the last enabled composer control), then
  wraps to the rail. A conversation sheet replaces the conversation stops;
  the browser list leads to the sheet, and its last control wraps to the rail.
  A narrow sheet cycles within its visible controls. The controller owns one
  detail stack: Back and Escape pop one page; × closes the whole stack.
  Every push records the focused opener, and every pop uses the same restoration
  path to that opener (or the nearest surviving control).
  Header order is Back (when depth > 1), More (profiles only), ×.
  Profile content follows with avatar, Chat, Rename or Add friend as available.
  Group info follows with avatar, Members, Notifications and Leave as available.
  Members follows with search, Clear (when present), then the member list.
  Recovery feedback never changes the stack's position or focus cycle.
  Shift+Tab is the exact reverse of each cycle.
- Messages are click-focusable for selection but not Tab stops. Slint 1.18
  cannot style links or inline images inside a selectable TextInput. Messages
  with those features use a measured flow: Select text in the context menu
  exposes the original body for native selection. Dragging non-link text in
  that flow does not select a range; dragging a link in a text-only message
  selects the whole body. An emoji message enters fallback selection only
  through its context menu. Cross-run drag selection remains a deviation.
- Selection (which chat is open) and focus (which control receives keys)
  are visually distinct and independent.
- Incoming data never moves focus. Completion of a send never moves focus.
- Opening a conversation focuses the composer. Opening a panel focuses its
  first control; closing it restores the opener. If narrowing hides the focused
  browser while a conversation is shown, focus moves to the composer.

### 8.3 Pointer

- Wheel and touchpad scroll the pane under the pointer; nothing spills into
  neighbours at boundaries.
- Scrollbar thumbs have a pointer target of at least 10 px and stay under the
  pointer during a drag; content changes are deferred until release.
- Single click on a conversation opens it. Single click on a friend opens a
  profile panel; nothing sends on a single click.
- Right click on a message selects it as the target and opens its menu
  without clearing an existing text selection.
- Dragging across a link selects text; click without drag activates it.

## 9. Feedback and text

### 9.1 Where feedback lives

| Failure or state | Location | Not acceptable |
| --- | --- | --- |
| Conversation list load/refresh | list footer item with Retry | status line |
| History load | strip inside the conversation header area; does not push history | row above history that shifts content |
| Older history load | item at the top of the history | banner |
| Send | mark and explanation on the message | status line, connection banner |
| Draft storage | caption under the composer with Retry; Send disabled | modal |
| Connection, authentication | quiet inline header state; recovery strip below headers for Offline/AuthExpired or an unavailable session | per-chat subtitles |
| Media | inside the media frame | row collapse |
| Settings save | beside the control | toast only |

There is no general status line. Counts and "Refresh to update" captions do
not belong in the shell.

### 9.2 Wording

- Button labels are verbs, at most three words: Send, Retry, Open chat,
  Leave group.
- Captions state a limitation only where the user meets it: the empty state
  of a search says what was searched; the top of the history says older
  messages are unavailable only when the cursor has ended; the Add friend
  destination is absent until it works.
- No permanent captions that explain how a control works ("Enter inserts a
  line") and no implementation words (synthetic, loaded, server confirmed)
  in production UI. Shortcut help lives in Settings.
- Delivery marks have accessible labels: Sending, Sent, Read, Not sent,
  Unconfirmed.
- Times: today → `HH:MM`; yesterday → "Yesterday"; this week → weekday; this
  year → `M/D`; older → `YYYY/M/D`. Date dividers: Today, Yesterday,
  weekday + date within a week, full date otherwise. Locale follows the
  system where the toolkit allows.

## 10. Visual model

- **Type scale** derives from the user's message text size: body = base,
  small = base − 2 (min 11), heading = base + 6, list title = base, list
  preview = base − 1, time = base − 3 (min 10). No hard-coded pixel sizes
  for text except the two minimums.
- **Spacing** uses 4/8/12/16/24 px steps. Row heights derive from the type
  scale, not from fixed constants.
- **Colour roles** come from the system palette (background, surface, text,
  secondary text, accent, selection, danger) and follow the system light/dark
  scheme. Brand green is used only for the accent role and outgoing bubbles.
- **States** hover, selected, focused, pressed, and disabled are each
  visually distinct and do not rely on colour alone (focus ring, weight).
- **Scrollbars** share one style across panes. In the software renderer a
  fixed gutter is acceptable; thumb width ≥ 10 px.
- **Rows**: avatar 40 px, name on the first line with time right-aligned,
  one preview line with unread badge or marker right-aligned. Long names
  elide; previews never wrap.
- **Bubbles**: text bubbles size to content up to 480 px or the pane width
  minus 140 px. Stickers have no bubble. Photos keep aspect ratio inside a
  bounded frame reserved before load. Time and delivery mark sit beside the
  bubble in a fixed slot.
- **Motion**: none that runs continuously. A scroll animation, if any, is
  interruptible and disabled under reduced-motion settings.
- **Icons**: one SVG family at 16/20/24 px, recoloured by role.

## 11. OS integration model (Linux, Plasma/Wayland first)

| Concern | Requirement | Mechanism |
| --- | --- | --- |
| Identity | Wayland app id, X11 class, desktop entry, and icon all `org.lineclient.Desktop`; window title "LINE" plus optional unread count, never the account name | `slint::set_xdg_app_id`, desktop entry `StartupWMClass`, `SingleMainWindow=true`, `StartupNotify=true`, `Keywords`, `Categories` |
| Single instance | A second launch activates the running window and exits; a launch with a URL or notification action is forwarded | Own well-known D-Bus name `org.lineclient.Desktop`; implement `org.freedesktop.Application` (Activate, Open, ActivateAction); `DBusActivatable=true` |
| Activation | Raising the window from a notification works under Wayland | Use the activation token from the notification daemon (`ActivationToken` signal) when the toolkit can consume it; fall back to `request_user_attention` so the task entry highlights |
| Notifications | Native desktop notifications with actions, replacement, withdrawal, per-app settings in Plasma | `org.freedesktop.Notifications`: hints `desktop-entry`, `category=im.received`, `urgency`, `image-data` (privacy level 1 only), `replaces_id` per conversation, `resident=false`, default action opens the chat |
| Sound | At most one sound per burst; follows the desktop sound theme and mute | Hint `sound-name=message-new-instant`; if the daemon ignores sounds, defer in-app playback to a later session rather than bundling audio |
| Badge | Unread conversation count on the task manager entry | `com.canonical.Unity.LauncherEntry` `Update` signal with `application://org.lineclient.Desktop.desktop`, `count` and `count-visible`; `progress` unused |
| Attention hint | Inactive window with new unread highlights in the task bar | winit `request_user_attention(Informational)` |
| Window state | Know when the window is inactive or hidden | winit `Focused`, `Occluded`; treat unknown as inactive |
| Background mode | Off by default: closing the window quits. When enabled in Settings, close hides to a tray item and sync continues | StatusNotifierItem (`org.kde.StatusNotifierItem`) with menu Open, Mute, Quit; preference explains that notifications continue |
| Autostart | Off by default, explicit preference | `~/.config/autostart/org.lineclient.Desktop.desktop` |
| Paths | No app state inside the repository | session → `$XDG_DATA_HOME/line-desktop`, drafts/outbox/prefs → `$XDG_STATE_HOME/line-desktop`, caches → `$XDG_CACHE_HOME/line-desktop`; `--state`/`--cache-dir` remain overrides |
| Theme and scale | Follow system light/dark, accent, font, and per-monitor scale | Slint `Palette.color-scheme`, default font from fontconfig, no custom DPI handling |
| Input method | Fcitx5 composition works in every text field | Compositor `text-input-v3` through winit; app never intercepts composition keys |
| Clipboard and links | Copy as text; open links with the default browser after deliberate activation; show the full URL on hover or in the menu | `xdg-open` via `open`-style helper off the UI thread |
| Files | Attachment picking and drag-and-drop use the desktop file chooser | xdg-desktop-portal file chooser; `text/uri-list` drops |
| Accessibility | Every control has a role, label, and state; selection and delivery state not colour-only | Slint `accessibility` feature (AT-SPI) |
| Privacy | No message text or names in logs, window titles, or crash output; notification history respects the preview level | existing logging rules |
| Lifecycle | Quit flushes drafts and stops sync; logout clears notifications and badge | existing flush path plus notification withdrawal |

## 12. Deliberate deviations from LINE Chrome

- Explicit Send with Ctrl+Enter is the default until Fcitx acceptance is
  complete; "Enter sends" is an opt-in preference.
- Text selection is per message in v1; cross-message selection is not a
  goal. Copy message and Copy selection are both offered.
- No tray and no background mode by default.
- The Add friend destination and group management are absent until their
  protocol operations exist; they are not shown as placeholder pages.
- Reactions are editable: hover/focus or React opens six choices (NICE, LOVE, FUN, AMAZING, SAD, OMG). The own choice is highlighted; choosing it again cancels. Changes are optimistic, revert on failure, and show one transient danger line.
- Profiles and group info open in the detail panel or sheet (§3.4), not in a
  separate popup window.

## 13. Dependencies on protocol work

The following requirements depended on missing worker data at the initial
2026-10-17 audit. Protocol sessions P1–P4 now provide sourced adapters; their
handoffs and ux-acceptance.md distinguish implemented and synthetic-tested paths
from account-verified behavior. Native mute remains unknown as described in §7.1.
The consuming UX sessions retain these dependencies.

- Live events (incoming messages, receipts, reads elsewhere, group events,
  reconnect catch-up) with reconnect state.
- Unread counts and mute flags per conversation; read marker submission.
- Older history paging with a per-chat cursor.
- Definite rejection versus uncertain result classification for sends.
- Inline emoji spans, reply structure, file and system message types.
