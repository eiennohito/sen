# Chromium UI reference and native reconstruction plan

Audit date: 2026-10-15. Reference: installed LINE Chrome extension 3.7.2.
This is a first mapping session, with explicit coverage gaps below. The native
sketch is not a recreation of this interface yet.

## Evidence and privacy

Live DOM inspection used Chrome's approved browser CDP connection on loopback
port 9222. The viewport was 800 × 580 CSS pixels at device scale 1.5. Static
source inspection supplements the live observations; source presence alone
never establishes that a feature works for this account.

Reference files, kept outside Git:

- `manifest.json`: SHA-256 `37374636a886e458ccbe1bf7195de7794c12131932a30270f8669d5c32a7eb42`
- `static/js/main.js`: SHA-256 `2912a06d868c2829636be1613c622f28807efe74a7868cfb143678a38b80cc2a`
- Crypto WASM: pinned separately in HANDOFF.md.

Only public control labels, component classes, geometry, and protocol structure
belong in this document. Do not commit screenshots of the account, profile
values, contact names, messages, account counts, identifiers, or routes containing
identifiers. Inspection opened navigation, menus, and settings; no message was
sent and no membership, profile, or preference was changed. The member popup was
closed; the settings window opened for inspection may remain open. Main navigation
was returned to Chats.

Chrome login invalidated the native session, according to the user. Do not
re-pair or make native authenticated requests during this audit. Performance
verification below uses synthetic fixtures without account/network access.

## Shell and layout — live

At the sampled size, the content root was 800 × 524 CSS pixels. The remaining
window area was not characterized. Treat these as observed dimensions, not a
fixed-size specification for the native app.

| Region | x | y | width | height |
| --- | ---: | ---: | ---: | ---: |
| Navigation rail | 0 | 0 | 62 | 524 |
| Chat categories | 62 | 0 | 738 | 52.92 |
| Conversation column | 62 | 52.92 | 328 | 471.08 |
| Selected conversation | 390 | 52.92 | 410 | 471.08 |
| Floating create button | 321 | 455 | 52 | 52 |

The conversation search region was 68 px high. Navigation buttons were 32 × 32
at x=15: Friend y=20, Chat y=74, Add friend y=128, Sound setting y=435, and
Popover more actions y=477. The computed body font was 16 px / 20 px with
platform sans-serif fallbacks including Hiragino Sans and MS PGothic.

Native design implication: establish a navigation rail, a stable list pane,
and an independently scrolling conversation pane. Resize these at useful
minimum widths; retain the selected conversation and draft across navigation.
Use a consistent SVG icon family, visible selection, hover/focus states,
accessible labels, and balanced spacing before adding decorative effects.

## Screen and interaction inventory

| Surface | Evidence | Observed behavior / controls | Native gap |
| --- | --- | --- | --- |
| Chats | Live | All, Friends, Groups, Official accounts; Search chat list; reset; floating Chat / Group creation controls | Only one undifferentiated list |
| Friends | Live | Search by display name; collapsible Groups category; selected conversation remains on right | No friends view |
| Add friend | Live | Friend recommendations category; selected conversation remains on right | No add-friend view; search dialog still needs live inspection |
| Navigation More | Live | Settings, Help, About LINE, Log out, Quit | Absent |
| Group header | Live | Name/member button, mute, pop-out, more | Title only |
| Group More | Live | Mute chat, Invite, Edit this Group, Report, Leave | Absent |
| Group/member window | Live | Group image/name edit controls, member search, profile actions, per-member leave controls, auto-add checkbox; separate window | Absent; permission/confirmation semantics untested |
| Composer | Live | Contenteditable; Send file, Capture screen, Select sticker | Plain TextEdit and explicit Send |
| Settings | Live | Separate window with six tabs, detailed below | Absent |
| Start chat / create group | Source + entry controls live | startChat, selectFriendPopup, selectedlist | Selection/submission flow still unobserved |
| Find friend / profile | Source | findFriendModal, profileModal | Live dialogs still unobserved |
| Sticker picker | Source + opener live | stickerPopup | Contents, categories, purchase/download states unobserved |
| Message menus / reaction results | Source | actionGroup, reactionPopover, reactionResultPopup | Need live context-menu and ownership-state mapping |
| Media viewer / attachment confirmation | Source | mediaViewPopup, sendFileModal | Need live viewer and file-selection mapping |
| Login / QR / PIN | Source | loginForm, loginQr, pinCodeModal | Current auth CLI exists; native flow not designed |

Opening a group name spawned a separate extension target, rather than an
in-page member panel. The inspector must enumerate/attach popup targets and
survive targets closing. Do not infer that a visible per-member control is
available to every group role; permissions and confirmations remain to map.

## Messages and history

Live component families included messageLayout, username, textMessageContent,
metaInfo, messageDate, stickerMessageContent, imageMessageContent, linkbox,
replyMessage, replyMessageContent, reactionBubblelist, reactionBubble,
reactionPopover, scrollDownButton, and notificationStateToast. These establish
coverage requirements, not an exact visual specification. File, video, audio,
contact, and system-message components are present in source but were not
opened and verified individually.

The current native message row loses sender identity, rich spans, reply
structure, and much metadata. Model those explicitly before restyling bubbles.
Preserve whitespace, links, multiline text, selection/copy, and accessible media
labels. Standard reactions need consistent assets rather than font-dependent
Unicode substitutes. Inline LINE emoji, stickers, reaction images, and ordinary
UI icons are separate asset/rendering concerns.

### Why inline emoji are missing — source trace

Chrome's decrypted JSON `REPLACE` field is copied into content metadata by its
decoder (`yL` in the inspected bundle). The native decoder currently carries
text/location, keyMaterial, and fileName, but discards `REPLACE`.
`REPLACE.sticon.resources` describes inline emoji; source helpers `LI` / `II`
build emoji resources and ownership metadata. Before implementation, confirm
whether all offsets are JavaScript UTF-16 indices using synthetic mixed
Japanese/astral-emoji fixtures. Resolve spans into text/image runs; do not
replace the entire text message with one thumbnail.

Sticker variants also use STK_IMG_TXT, STKOPT, STKHASH, STKPKGID, STKID, STKVER.
Current caching covers static stickers/photos/paid reactions only. Animated,
custom-text, and composite sticker behavior remains unimplemented. Chrome's
standard reaction assets are `icon_1001_32x32` through `icon_1006_32x32`; this
inventory does not copy proprietary assets into the repository.

### Older history — source contract, live scroll still pending

Chrome uses `getPreviousMessagesV2WithRequest` with positional arguments:

```text
[
  {
    messageBoxId: chatMid,
    endMessageId: {
      deliveredTime: oldest.deliveredTime ?? oldest.createdTime,
      messageId: oldest.id
    },
    messagesCount: configuredCount
  },
  syncReason
]
```

Its merge removes an inclusive cursor duplicate before prepending. Native code
currently fetches only 50 recent messages and has no delivered-time cursor or
older-history command. Increasing that limit is not pagination.

Implement a per-chat cursor, loading/end/error states, ID deduplication, and
anchor preservation when prepending. A late page must not replace a different
chat. Keep decoded history and retained images bounded; virtualize rows before
allowing long histories. Live follow-up must measure the trigger threshold,
spinner/end treatment, anchor position, and return-to-latest behavior.

## Settings — live

| Tab | Controls observed | Coverage limits |
| --- | --- | --- |
| Basic | Profile photo/name/status edits; language selector | Values omitted; edits not submitted |
| Notifications | Pop-up notification, preview, sound; rules for new messages, mentions, group invitations/new members; suppress mobile notifications while using Chrome | OS permission flow not exercised |
| Chats | Font Small/Normal/Large/Extra large; Enter or Alt+Enter send shortcut; chat folders; Original/Standard photo resolution; drag-drop file previews; URL previews; GIF autoplay; block nonfriend messages; hidden chats | No preferences changed |
| Friends | Blocked accounts: Unblock/Delete; Hidden accounts: Unhide/Delete | No actions performed |
| Keyboard shortcuts | Global navigation and chat/media shortcuts below | Labels inspected, shortcuts not invoked |
| About LINE | Tab opens | Body/version links not captured |

Keyboard labels: Ctrl+W close, Ctrl+1 Friends, Ctrl+2 Chats, Ctrl+3 Add friends,
Ctrl+Shift+F search, Shift+Enter new line, Ctrl+F chat search, Ctrl+E stickers,
Ctrl+O file, Ctrl+P capture, Ctrl+S save file, Ctrl+Plus/Minus image zoom.
Native shortcuts must defer to IME composition and focused text widgets.
Retain the safe explicit Send default until Fcitx acceptance is complete.

## Protocol inventory — source, not executed

Friends: getAllContactIds, getContactsV2 with
`{targetUserMids, neededContactCalendarEvents: []}`; blocked, hidden, and
recommendation APIs require a further source-contract pass.

Groups: getAllChatMids, getChats with members/invitees, createChat, updateChat,
inviteIntoChat, cancelChatInvitation, acceptChatInvitation,
rejectChatInvitation, deleteSelfFromChat. Model membership/role state and
confirmation/error handling before exposing mutating controls.

## Performance work completed during this audit

Offline synthetic V2 fixture, release build, 20 decryptions: constructing a
channel for each message cost 140.15 ms/message; reusing it cost 4.54 ms/message.
Runtime initialization was 170.31 ms. This isolates a crypto bottleneck; it is
not an end-to-end app speedup or a group/network benchmark. Reproduce with:

```sh
cargo run --release --offline --bin line-wasm-poc -- --bench
cargo test --release --offline --lib cached_channel_is_reusable_across_messages -- --ignored
```

The client now reuses up to 64 crypto channels, keyed by conversation and key
IDs (plus sender for groups), destroying evicted channels and discarding all
pointers when the runtime resets. Public keys remain separately bounded.

Decoded-message memoization is fingerprinted on the complete raw message,
including ciphertext and reaction metadata. Recent pages persist encrypted
and authenticated under an account-scoped directory. Selection uses a cached
page; explicit Refresh and post-send reload fetch from the server and reuse
unchanged decoded messages. No automatic receive or background refresh exists:
a cached view can be stale until Refresh. Cache access still requires the
normal authenticated client startup; this is not offline account startup.

Asset arrivals now update affected Slint rows instead of replacing the entire
chat/message model and rebuilding every reaction model for each image.
Native account timing/CPU/RSS after these changes is still pending because
Chrome owns the active login.

## Reconstruction sequence and acceptance

1. Complete the unobserved flows above with Chrome, retaining privacy-safe
   structure only. Record empty/loading/error/permission states as well as
   populated views. Measure menus, message rows, and narrow-window behavior.
2. Build shell navigation, list hierarchy, typography, SVG icons, states, and
   spacing in the synthetic Slint preview. Friends and Groups need real models,
   search, categories, selection, and explicit account-command boundaries.
3. Implement paged history and rich message rows, including sender, replies,
   timestamps, inline emoji, stickers, links, and reactions. Preserve viewport
   and drafts; bound memory while browsing older pages.
4. Add member/group/profile/settings surfaces from the evidence matrix. Keep
   actions visibly disabled until their protocol behavior is implemented.
5. Validate screenshots at the reference size plus normal desktop sizes through
   Slint MCP. Measure cold/warm selection, refresh, 1000-message paging, CPU,
   RSS/PSS, and image-cache reuse. Test real Fcitx preedit/candidates manually.

Do not describe the UI as recreated, polished, or feature-complete until these
checks pass. The existing UI is still the initial sketch.

## Repeatable inspection

`node tools/chrome-ui-map.mjs --interactive` uses the browser endpoint from
`~/.config/google-chrome/DevToolsActivePort`; the newer Chrome mode need not
serve `/json/version`. Approve Chrome's debugger prompt when shown. Keep one
connection: `:targets` lists redacted extension targets, `:attach TARGETID`
switches windows, `.exit` detaches. Default mode prints component/navigation
structure. Arbitrary evaluation is powerful: the tool does not enforce read-only
behavior or redact arbitrary expressions. Never query account storage or dump
whole DOM/text; filter private descendants before inspecting controls.
