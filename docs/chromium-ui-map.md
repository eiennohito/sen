# Chromium UI reference and native reconstruction plan

Audit date: 2026-10-15. Reference: installed LINE Chrome extension 3.7.2.
This is a first mapping session, with explicit coverage gaps below. The native
sketch is not a recreation of this interface yet.

Use [Native messaging UX acceptance checklist](ux-acceptance.md) alongside this
reference when implementing screens. It specifies proposed native scrolling,
focus, navigation, sending, unread, notification, and recovery behavior, with
interaction scenarios to verify. Observations in this map do not by themselves
define those native behaviors.

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

## Everyday-use requirements — user feedback

The user explicitly considers the current UX acceptable only as a proof of
concept, not for daily messaging. Their reported problems are examples, not an
exhaustive defect list. Audit complete workflows and their changing states,
including keyboard, pointer, selection, scrolling, asynchronous updates, errors,
and recovery; a static screen inventory or screenshot comparison is insufficient.
The requirements below are user feedback and design acceptance criteria, not
additional claims of observed Chrome behavior.

### Outgoing message lifecycle and read receipts

Model the sequence explicitly: send requested → waiting for server confirmation
→ server-confirmed message → potentially later read notification. Server
confirmation must never imply that a recipient has read the message. Lack of a
read notification is not a send failure, and a read receipt can arrive much later.

Keep per-message state rather than relying on a global “Message sent” status.
Reconcile a local pending item with the server-assigned identity so confirmation,
history reloads, and live operations do not create duplicates. Show pending,
confirmed, and read states with stable placement and accessible descriptions.
Represent definite failure separately from an ambiguous timeout; preserve the
draft/recovery path and never automatically resend an ambiguously accepted item.

The next source/live audit must trace send-response fields, operation/event
subscription, read-receipt identities or watermarks, ordering, reconnect/catch-up,
and persistence. Verify direct-chat versus group semantics; show reader counts
only when supported by actual protocol evidence. Specify how opening a chat,
visibility, scrolling, and focus affect marking incoming messages read. Do not
emit read acknowledgements merely to inspect these flows during the audit.
Test confirmation-before-receipt, delayed/duplicate/out-of-order receipts,
reconnect replay, switching chats while pending, and restart reconciliation.
Native asynchronous read-receipt handling is not implemented yet.

### Message selection and copying

The current native view does not allow selecting/copying message text. Provide
mouse selection, keyboard selection/copy, and an accessible Copy action. Define
copy behavior for multiline text, links, replies, inline emoji, and mixed media;
preserve meaningful text and line breaks. Selection must coexist with scrolling,
context menus, and incoming updates without being cleared by image arrivals.
Verify cross-message selection behavior against Chrome and choose/document the
native behavior. Copying visible message text must not require entering an editor.

### Density, previews, and media presentation

The user reports cramped conversation/message previews appearing on top of one
another. Inspect both list previews and message rows at narrow widths and HiDPI:
use clear name/preview/metadata hierarchy, appropriate line height, constrained
preview lines, stable unread/time placement, and enough vertical separation.
Long names, multiline previews, missing images, font fallback, and large text
must not overlap adjacent content or obscure controls.

Images and stickers currently consume too much space in their backgrounds.
Use content-aware rows: text bubbles, image bounds/aspect ratios, transparent
stickers, and system/date items need different presentation. Avoid a universal
full-width or oversized bubble. Preserve sticker transparency and media aspect
ratios, bound previews, and place timestamps/read state without an empty padded
card around every asset. Some message types need no bubble/background at all.
Measure spacing with mixed text/photo/sticker/reply/reaction sequences and test
loading/error states so completed media does not cause disruptive layout jumps.

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

Read-only members (P5a): the repository has no local `dumps/`, HAR, or captured
getChats response. The existing client and synthetic parser fixtures support
`extra.groupExtra.memberMids` as a map (MIDs are keys) and room `memberMids` as
an array. There is no local trace establishing roles, join times, or map-value
semantics. The map's existing count is retained; the legacy room array adds
self exactly once. Invitees are excluded and absent membership stays unknown.

The existing requests are `getChats` with
`[{chatMids, withMembers: true, withInvitees: false}, 0]` and the sender/contact
path's `getContactsV2` with
`[{targetUserMids, neededContactCalendarEvents: []}, 0]`. Contact responses use
`contacts[mid].contact`, with `displayNameOverridden`, `displayName`,
`statusMessage`, and `picturePath`. Member resolution uses those existing
shapes on the account worker in batches of at most 100 MIDs. Membership sets
are retained for up to 128 groups/rooms. A separate 4096-entry member contact
cache holds answered identities and server-answered absence; call failures
cache nothing and remain eligible for panel Retry. It never evicts friend or
1:1 contacts. Unknown identities show an initial and “Unknown member”; history
keeps its sender fallback and can retry failed/absent identity lookups. Self's
identity comes from the already-fetched account profile.

There is no local trace identifying membership operation numbers, and no
join/leave/removal/invitation decoder. The current decoder handles messages
25/26/43, watermark 40, receipt 55 and withdrawal 64/65. Unrecognised operations
already schedule chat-list/details refresh, and full sync fetches chat details.
Those getChats responses replace retained member sets; an open list refreshes
in place, including equal-count replacements. Freshness follows this existing
chat refresh coverage and paging scope. No incremental membership decoding or
live group/room compatibility is claimed. Member profiles allow Open chat only
for a known friend or an existing conversation; mutation/invitation flows remain
outside the read-only implementation.

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
   populated views. Trace send confirmation, asynchronous read receipts, incoming
   read acknowledgement rules, and reconnect behavior. Measure menus, message
   rows, and narrow-window behavior; inspect selection/copy and media spacing.
2. Build shell navigation, list hierarchy, typography, SVG icons, states, and
   spacing in the synthetic Slint preview. Friends and Groups need real models,
   search, categories, selection, and explicit account-command boundaries.
3. Implement paged history and rich message rows, including sender, replies,
   timestamps, inline emoji, stickers, links, and reactions. Preserve viewport
   and drafts; bound memory while browsing older pages. Include selectable text,
   per-message sending/confirmed/read states, legible previews, and content-aware
   media backgrounds. Treat the user-reported issues as a starting point.
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

## Protocol source trace — 2026-10-17 (not live validation)

The installed 3.7.2 bundle's operation enum maps SEND_MESSAGE=25,
RECEIVE_MESSAGE=26, SEND_CHAT_CHECKED=40, SEND_CONTENT=43,
NOTIFIED_READ_MESSAGE=55, NOTIFIED_JOIN_CHAT=60, NOTIFIED_LEAVE_CHAT=61,
DESTROY_MESSAGE=64, NOTIFIED_DESTROY_MESSAGE=65. `RS` interprets checked
`param1` as chat and `param2` as lastSeenMessageId, clearing unreadCount;
`yS` interprets receipt `param1/2/3` as chat/member/endMessageId, with
createdTime as endTime. These are per-member range endpoints, not a message's
reader count. Source `getMessageReadRange` arguments are `[chatIds,syncReason]`;
`sendChatChecked` arguments are `[reqSeq,chatId,messageId,0]`.

The SSE constructor uses native EventSource `withCredentials:true`, no
custom signed-POST header mapper. Native SSE therefore sends the lct cookie,
Accept event-stream and version/language query; signed headers remain on thrift
and refresh POSTs. This requirement is source evidence, not a confirmed gateway
header requirement. Ping has 20 s interval plus 10 s spare. Stream data remains
private and never enters diagnostics. Full/partial sync falls back to refreshing
the loaded native chat/message scope before advancing a supplied nextRevision;
profile/configuration categories outside the native model remain unimplemented.

`renewToken` calls POST `/api/auth/tokenRefresh` with `{refreshToken}` and
merges the returned token fields into tokenV3IssueResult before persisting.
The native path performs one refresh on SSE HTTP 401/403; missing/rejected
refresh becomes AuthExpired without pairing or resending. Scheduled proactive
refresh and the extension's AUTH_RETRY_REQUIRED jitter policy are not yet
implemented. No native request was made for this trace.

Mute is **not a server chat flag in the inspected code**: `VT` reads
`isMute` from user storage `messageBox[id]`, and the menu writes it through
`Fz(np.USER,"messageBox",id)`. This cannot establish a native server mute value;
unknown remains unknown, with native private local overrides supplied separately.

The source request allocator `HO` starts from crypto-random positive 31-bit
state, then increments. `kS` uses operation 25's reqSeq to replace the matching
`local-{reqSeq}` item. Native reconciliation additionally persists a hash of
the exact encrypted envelope (to/from/contentType/chunks) before transmission,
so a different device's matching small reqSeq cannot confirm an unrelated item.
Recent-page catch-up recovers the same binding by envelope hash even when the
operation predates the restart revision. Bindings are retired only after the
outbox state is durable. Legacy unresolved rows without a request identity cannot
be guessed into a match.

File download source `QO`/`zD` builds the bare OBS path: encrypted content uses
SID/OID (defaults m/message id), legacy content uses `/r/talk/m/{messageId}`;
ENC_KM selects authenticated media decryption and X-Talk-Meta. Native explicit
Save fetches at most 16 MiB, supports this constructed known-host route, and
reports failure for external DOWNLOAD_URL variants rather than trusting an
arbitrary URL. This is source-only implementation evidence, not a live download.

The source reader-count reducer explicitly rejects IDs `<= startMessageId` or
`> endMessageId`: read coverage is **start-exclusive, end-inclusive**. The initial
`endMessageId="1"` range is unadvanced and covers no messages; native typed
ranges preserve this sentinel (`open_end`) and do not turn it into infinity.

Chrome's unread-participation helper JO tests contentMetadata.SKIP_BADGE_COUNT
against the literal string "true". Native decoding preserves this flag separately
from message kind and withdrawal state: counted system rows participate, skipped
ordinary rows do not. Legacy cached metadata is refreshed (schema version 2); if offline, cached
text remains visible while the transient UI eligibility flag suppresses unread
participation until authoritative metadata arrives. The synthetic UI flag is
never written back to protocol cache. Withdrawal operations request a
fresh chat count rather than guessing a decrement.
