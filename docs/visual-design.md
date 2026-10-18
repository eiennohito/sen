# Visual design

Written 2026-10-18 by the coordinator from an audit of every screen (headless
captures at 980×720, 1280×900 and 420×760, light and dark). It replaces the
PoC look. Behaviour is [ux-model.md](ux-model.md); §10 there sets the type
scale and spacing steps, and this file sets everything else. The reference is
LINE's own client: quiet neutral surfaces, green for the brand, grey and green
bubbles, icons rather than words in toolbars. Implementations follow this file
literally. Anything it doesn't cover is asked about, not invented.

## What the audit found

- Fluent defaults instead of a design. The selected chat row is a solid blue
  fill, inputs and buttons are boxed Fluent controls, and the accent is blue.
- Text glyphs used as icons: `▶ Video`, `♫ Voice message`, `♙ Contact`,
  `▤ File`, `◇ Unknown message`, `ⓘ`, `⊘`.
- Full-width text buttons used for navigation ("Close profile", "Close
  members", "Close photo", "Open chat", "Start sign-in"), and a viewer toolbar
  made of a row of text buttons.
- Black 10 px scrollbars, and a black-bordered box for the connection strip,
  with its action button inline.
- White pill badges containing "•" on every chat for stale unread.
- A boxed composer with Send below it, plus a permanent caption ("Messages can
  be sent when connected").
- A header with no avatar and no actions. Profiles with no layout (see §3.4).
- A sign-in screen made of paragraphs and implementation vocabulary ("Chrome
  device slot", "encrypted drafts"), with full-width buttons and no visual
  centre.

## Tokens

Defined once in `app/ui/tokens.slint`. Nothing reads `Palette` directly
except tokens.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `bg` | #FFFFFF | #1E1F22 | conversation background, dialogs |
| `surface` | #F5F6F7 | #26272B | rail, browser pane, header strip background |
| `surface-2` | #EDEEF0 | #32343A | incoming bubble, input fields, chips, date pills |
| `surface-hover` | #E9EAEC | #2E3035 | row and button hover |
| `surface-selected` | #E2E4E7 | #383A40 | selected row, selected settings tab |
| `divider` | #E1E3E6 | #3A3C42 | 1 px lines |
| `text` | #111111 | #ECECEC | |
| `text-secondary` | #6B6F76 | #A3A7AE | times, previews, captions (≥4.5:1 on `bg` and `surface`) |
| `accent` | #06C755 | #06C755 | badges, the selected rail icon, switches, the enabled Send icon |
| `accent-strong` | #00873C | #3DDC84 | text buttons, links, focus ring, primary button fill (light) |
| `on-accent` | #FFFFFF | #0B2614 | text on `accent` and `accent-strong` fills |
| `bubble-out` | #C7F2A4 | #2F5D3A | outgoing bubble |
| `danger` | #D93025 | #FF6B5E | errors, destructive buttons |
| `warning-surface` | #FFF4D6 | #3A3220 | connection strip |
| `scrim` | #00000066 | #00000099 | behind dialogs and sheets |
| `viewer-bg` | #0E0E10 | #0E0E10 | media viewer |

Radii: bubbles 16 px, cards and dialogs 12 px, buttons and inputs 8 px,
pills and avatars fully round. Elevation is used only for dialogs, menus and
the side panel: a 1 px `divider` border plus the platform shadow. Panes
don't use shadows.

Focus: a 2 px `accent-strong` ring at 2 px offset on every focusable control,
following the control's radius. Focus never uses a fill change alone.

Icons: one SVG family (the existing `app/ui/assets/icons`, extended), 20 px in
toolbars, 24 px on the rail, recoloured by role. No Unicode glyph appears as
an icon anywhere. Needed additions: video, voice, file, contact, location,
info, withdrawn, unknown, sticker, attach, send, close, back, more, search,
members, zoom-in, zoom-out, fit, actual-size, previous, next, save, check,
check-double, clock, warning, error.

## Components

**Buttons.** Height 32 px, horizontal padding 16 px, radius 8 px, label
weight 600.

- Primary: `accent-strong` fill with `on-accent` text.
- Secondary: `surface-2` fill with `text`.
- Text button: `accent-strong` text with no fill.
- Danger: `danger` text, or a fill in confirmation dialogs.
- Icon button: 32 px square with a 20 px icon, radius 8 px, a hover fill and a
  tooltip, its label used as the accessible name.

Buttons are never full width, except in the bottom action bar of a narrow
sheet. Disabled controls use 40 % opacity and are never styled like links.

**Inputs.** `surface-2` fill, no border, radius 8 px, height 36 px, 12 px
padding, and a leading 16 px search icon in search fields. Placeholders use
`text-secondary` and hide while the field has focus (C6). On focus, the field
gets the focus ring.

**Scrollbars.** An overlay thumb: 6 px wide (10 px while hovered or dragged),
fully round, `text-secondary` at 45 % opacity, inside a 12 px hit gutter with
no track colour. Visible while scrolling or hovered, and fading out after 1 s.
In the software renderer the fade may be an instant hide.

**Chips (Chats filter).** A segmented row of pills, 28 px high, 12 px padding.
Unselected: `surface-2` with `text`. Selected: `text` fill with `bg` text. No
borders.

**Badges.**

- Unread count: an `accent` pill, 20 px high, min width 20 px, bold small
  `on-accent` text; above 999 it shows "999+".
- Stale unread: an 8 px `accent` dot. Never a pill with a dot inside.
- Muted chats show the count in `text-secondary` fill.
- A failed send in a chat shows a 16 px `danger` error icon in place of the
  badge.

**Avatars.** Always circular, 44 px in the chat list and friends list, 32 px
in the header and for group senders, 40 px in member rows, 96 px in profiles.
The fallback is the first grapheme on a tint derived from the id (eight
muted hues at text contrast), never the accent blue.

## Screens

**Rail** (56 px, `surface`). 24 px icons, 44 px hit squares. The selected
destination gets an `accent` icon on a `surface-selected` rounded square. No
left bar indicator. The badge sits on the Chats icon's top-right corner. The
account avatar (32 px) is at the bottom above Settings and opens your own
profile (U2).

**Browser pane** (`surface`).

- The title ("Chats" / "Friends") at heading size in `text`, not accent.
- Search field, then chips, then the list, with 12 px side padding.
- Chat row, 68 px high:
  - avatar;
  - name (weight 600, elided) with the time right-aligned in small
    `text-secondary`;
  - preview in `text-secondary`, one line, with the badge right-aligned;
  - a mute icon after the name when muted.
- Selected rows use `surface-selected` with radius 8 px and 4 px inset, and
  keep `text` colours. Hovered rows use `surface-hover`.
- Friends: the "Groups" and "Friends" headers are small weight-600
  `text-secondary` labels with the chevron on the right and a count ("Friends
  12"). Rows are like chat rows, with the status message as the preview.

**Conversation header** (56 px, `bg`, 1 px `divider` bottom).

- Back (narrow only), the avatar, then the name (weight 600, base + 2) with a
  subtitle in small `text-secondary` ("25 members" for groups, nothing for 1:1
  chats).
- The avatar and name are one button that opens the profile or group info.
- Right side, icon buttons: Members (groups only), then More (⋯), which holds
  the profile or group info item.

**Connection strip.**

- Full width directly under the header: 36 px, `warning-surface`, 16 px
  padding, a warning icon, text in `text`, and an optional secondary button
  right-aligned ("Sign in again").
- No border.
- Wording: "Offline. Reconnecting…", "Connecting…", "Sign-in expired".

**History** (`bg`).

- Incoming bubble: `surface-2`. Outgoing: `bubble-out`. No borders. Padding
  8×12, radius 16, with consecutive bubbles from one sender 2 px apart and
  12 px between senders.
- Group sender: a 32 px avatar on the first bubble of a run, and the name in
  small `text-secondary` above it.
- The time and delivery slot sit beside the bubble's bottom edge in small
  `text-secondary`. Delivery icons: clock (Pending), check (Sent),
  check-double in `accent-strong` (Read), warning (Uncertain), error in
  `danger` (Rejected). Each icon is named in the accessible label.
- Date separators and system messages ("Message unsent", "Aoi joined the
  group", "Beginning of conversation") are centred pills: `surface-2`, small
  `text-secondary`, padding 4×12, with a leading 14 px icon where meaningful
  (withdrawn, info). No glyphs.
- Reply quote: inside the bubble, a 3 px `accent-strong` left bar, the sender
  in small weight 600, and one line of text in `text-secondary`, on a 6 %
  `text` tint.
- Rich content cards replace the glyph rows. Each is a bubble containing a
  40 px rounded icon tile (`bg` at 60 % on the bubble colour, with the type
  icon), a title (file name, "Video", "Voice message", contact name,
  location name) and a meta line ("2.0 KB", "2:05", "0:12").
  - Files get a Save icon button on the right.
  - Unknown content: an unknown icon with "Unsupported message".
  - Undecryptable: a lock icon with "Couldn't decrypt this message".
- Photos: radius 12, no bubble, reserved frame. Stickers: no bubble, 120 px
  box.
- Failed send: under the bubble, one line of small `danger` text ("Not sent"
  / "Maybe not sent") followed by text buttons on the same line: Retry (or
  Resend) and Discard. Copy is in the context menu only.
- Latest button: a 36 px round `bg` button with a shadow, a down arrow and a
  count badge, 16 px above the composer and right-aligned.

**Composer** (`bg`, 1 px `divider` top, 8 px padding).

- Left: icon buttons for Sticker (P2), then Attach (only once P3 exists).
- Centre: a field with `surface-2` fill, radius 18, no border, padding 8×12,
  growing from 1 to 6 lines.
- Right: Send as an icon button. The icon is `accent` when enabled and
  `text-secondary` when disabled. Its accessible name is "Send".
- No captions. When offline, Send is disabled and the strip explains why.

**Context menu.** `bg` with a 1 px `divider` border, radius 8, 4 px padding,
32 px items with 12 px padding, and a hover fill. Destructive items (Unsend)
use `danger` text. Shortcut hints are right-aligned in `text-secondary`.

**Dialogs.**

- `bg`, radius 12, padding 24, width 400 px (Settings: 640 px), over the
  `scrim`.
- The title is heading size, with an × icon button top-right.
- Body text in `text`, then buttons right-aligned: primary last, Cancel as
  secondary.
- Settings uses a left tab list (selected: `surface-selected`, `text`, radius
  8) with the content on the right.

**Detail panel / sheet.** Follows ux-model §3.4: a header with ← and × icon
buttons, the cover band in a 25 % `accent` tint, and so on. Panel 320 px,
`bg`, with a 1 px `divider` left border.

**Media viewer** (`viewer-bg`, full window).

- A top bar overlaid on the image with a black-to-transparent gradient. On
  the left, × Close, then the file name and dimensions in white small text. On
  the right, icon buttons in white: zoom out, zoom in, fit, 1:1, Save.
- Previous and next are 40 px round icon buttons, vertically centred at the
  left and right edges, shown only when there is a target.
- No text buttons and no debug outline.

**Sign-in** (full window, `bg`).

- A centred column, 360 px wide:
  - a "LINE" wordmark in `accent`, at heading + 8;
  - the title "Sign in";
  - one line in `text-secondary`: "Scan the code with LINE on your phone.";
  - the QR code at 220 px on white with a 16 px quiet zone and a 12 px radius
    card;
  - a status line under it ("Waiting for your phone…", "Enter 1234 on your
    phone", "Signed in").
- A primary Start / Try again button, and a text button Back.
- One `text-secondary` caveat line under the buttons: "Signing in here signs
  LINE out in Chrome."
- No mention of encrypted drafts, sessions or device slots.

**Empty states.** Centred in the pane: a 48 px icon in `text-secondary`, one
line in `text`, and optionally one line in `text-secondary`.

- No search results: "No results".
- Chat with no messages: the 96 px avatar, the name and "No messages yet".
- Friends empty: "No friends yet".

## Flows added after the audit

**Reactions.** Hovering or focusing a bubble shows a 28 px round
reaction button beside the time slot, and the context menu has React.
Either opens a horizontal popover above the bubble: radius 20, `bg`, the menu
border, six 32 px reaction buttons (NICE, LOVE, FUN, AMAZING, SAD, OMG) drawn
as the existing Unicode substitutes until reaction images exist. The current
own reaction gets a `surface-selected` circle. Picking it again cancels the
reaction (`cancelReaction`). Reactions under a bubble are a pill (`bg`, 1 px
`divider`) with up to three reaction icons and a total count. Own reactions
are applied optimistically and reverted, with one transient danger line,
when they fail.

**Sticker picker.** The Sticker icon button in the composer opens a popover
anchored above the composer: 360×320 px (in narrow layouts the full width at
the same height), radius 12, `bg` with the menu border.

- Top: a horizontal row of 36 px package tabs, each showing the package's
  first sticker. The selected tab gets `surface-selected`.
- Body: a 4-column grid of 72 px stickers with a hover fill, scrolling.
- Activating a sticker sends it at once (LINE behaviour) and closes the
  popover. Escape closes it.
- Stickers are images from the media cache: packages that are already
  loaded open with no delay.
- "No stickers" is the empty state.

**New chat.** A New chat icon button (pencil-square) at the right of the
"Chats" title. It opens a dialog (Dialog component, 400×560):

- search field;
- the friends list with checkboxes (40 px avatars);
- the selected friends as removable chips above the list;
- with exactly one friend selected, primary "Chat", which opens or selects the
  1:1 chat with no RPC;
- with two or more, a name field (placeholder: the member names) and primary
  "Create group" (`createChat` type 0). Its progress label is "Creating…".
  Failure gives one danger line in the dialog. Success opens the new group.

**Attachments (P3).** An Attach icon button (paperclip) after Sticker in
the composer opens the system file chooser (XDG portal) for multiple files.
Pasting an image (Ctrl+V with image data, or file URIs) or dropping files on
the conversation does the same without the chooser. Nothing is transmitted
on selection, paste or drop (CMP9). It opens the attachment dialog
(Dialog, 480 px wide):

- The title is "Send to <chat name>".
- A list of the chosen items, each 64 px high: a 56 px thumbnail (photos) or
  the file-type icon tile, then the name and size, then a × remove icon
  button.
- Photos over 10 MB, or files over the server limit, show one danger line
  in the row ("Too large") and are excluded.
- Buttons: Cancel and primary "Send N", disabled when nothing sendable is
  left.

Sending creates one pending row per item, in order, through the normal send
lifecycle. Photo rows show the local thumbnail with a thin progress bar
(`accent`) along the bottom edge until the upload finishes. File rows show
the file card with the same bar. Failure gives the usual "Not sent" line with
Retry and Discard. Retry re-uploads from the original path. A file that no
longer exists turns Retry into "File missing".

## Not in scope here

- Animations, beyond the scrollbar fade and the hover fill.
- Custom fonts: the system UI font stays.
- Any theme other than the light and dark schemes from the system.
