# Protocol traces (LINE Chrome 3.7.2, 2026-10-18)

Captured live from the user-driven Chrome extension with
`tools/capture-line-protocol.mjs` (two passes). Shapes only: `<str:N>` is a
string of N characters, `<digits:N>` a numeric string, `<num>` a large number,
`<mid>`/44-character strings are opaque encrypted MIDs as returned by the API.
The raw shape traces and reference screenshots are local in
`artifacts/captures/2026-10-18T07-*` (ignored by Git; screenshots contain
account content). All RPCs are `POST https://line-chrome-gw.line-apps.com/api/talk/thrift/<Service>/<method>`
with a JSON array body, unless a full URL is given; responses are
`{"code":0,"message":"OK","data":…}`. Method names hidden by the first pass
were recovered from the extension's own path table (`SD("…")` in `main.js`).

Not captured (named in `main.js`, shapes unknown): `inviteIntoChat`,
`deleteOtherFromChat`, `cancelChatInvitation`, `acceptChatInvitation`,
`rejectChatInvitation`, `blockContact`, `unblockContact`, `findAndAddContactsByMid`.
A reaction from another user (operation 140) was not observed: the stream that
carried it predated the capture attachment.

## Operation types seen on `/api/operation/receive`

From the extension's `OpType` enum; ✓ = observed in this capture.

| Type | Name | Payload |
| --- | --- | --- |
| 4 ✓ | ADD_CONTACT | `param1` contact MID, `param2` "0" |
| 25 ✓ | SEND_MESSAGE | own message echo (`message`), also system message after `updateChat` (`contentType` 18, `LOC_KEY` "C_PN") |
| 26 | RECEIVE_MESSAGE | |
| 41 ✓ | SEND_CHAT_REMOVED | `param1` chat MID, `param2` time, `param3` "0" (after leaving) |
| 43 ✓ | SEND_CONTENT | media/file message once its content is stored (same message, `reqSeq` 0) |
| 49 ✓ | UPDATE_CONTACT | `param1` contact MID, `param2` "2" (setting flag: display-name override) |
| 64 ✓ / 65 | DESTROY_MESSAGE / NOTIFIED_ | `param1` chat MID, `param2` message id, `param3` "0" (unsend) |
| 121 ✓ / 122 | UPDATE_CHAT / NOTIFIED_ | `param1` chat MID, `param2` attribute flag ("1" observed for the notification setting) |
| 123–126 | INVITE_INTO_CHAT, CANCEL_CHAT_INVITATION (+NOTIFIED_) | |
| 127 ✓ / 128 | DELETE_SELF_FROM_CHAT / NOTIFIED_ | `param1` chat MID |
| 129–133 | ACCEPT/REJECT_CHAT_INVITATION, DELETE_OTHER_FROM_CHAT (+NOTIFIED_) | |
| 139 ✓ | SEND_REACTION | `param1` message id, `param2` JSON string `{"chatMid", "curr":{"predefinedReactionType":N}}` |
| 140 | NOTIFIED_SEND_REACTION | expected same shape as 139 for others' reactions (inferred) |

## Reactions (T5)

`TalkService/react` `[{"reqSeq":<num>,"messageId":"<digits:18>","reactionType":{"predefinedReactionType":2}}]` → `{}`.
Live: operation 139 (own, any device), 140 (others, inferred). Message pages
carry `reactions: [{fromUserMid, atMillis, reactionType:{predefinedReactionType | paidReactionType:{productId, emojiId, resourceType, version}}}]`.

## Official-account images (P1)

OA image messages (`contentType` 1, `toType` 0, no `chunks`, no `SID`/`OID`)
carry `contentMetadata` keys `PREVIEW_URL`, `DOWNLOAD_URL` (both on
`manager.line-scdn.net`), `PUBLIC`, `BOT_CHECK`, `BOT_TRACK`,
`NOTIFICATION_DISABLED`, `seq`. Chrome fetches `PREVIEW_URL`
(`https://manager.line-scdn.net/<id>/clearpreview`) directly, without the obs
route. The current client builds `obs.line-apps.com/r/talk/m/<id>/preview` for
these and gets 404.

## Stickers (P2)

Send: `TalkService/sendMessage` `[<reqSeq>, {"from","to","toType":0,"id":"<str:15 local>","createdTime","sessionId":0,"contentType":7,"contentMetadata":{"STKPKGID":"617","STKID":"<digits>","STKTXT":"<str>","STKVER":"9"},"hasContent":true}]`
(no E2EE chunks) → the stored message with server `id`. Echo: operation 25.

Picker data: `ShopService/ShopService/getOwnedProductSummaries`
(`POST …/api/shop/thrift/ShopService/ShopService/getOwnedProductSummaries`,
`["stickershop", offset, 1000, {"language":"<mapped locale first 2 letters>","country":"<profile regionCode>"}]`) → `productList[]`, `offset`, `totalSize`.
Start at offset 0, add 1000 while `offset + productList.length < totalSize`.
We choose to exclude expired `validUntil` (epoch milliseconds) from the picker;
the extension displays those packages with an expired status. -1 means unlimited,
and a missing expiry is not expired (extension `ez`: NaN is not expired).
Language follows extension `aw(navigator.language)`: normalize case and underscores,
match exactly, then match the first supported language prefix, else use en-US.
Supported locales in order: en-US, ja-JP, ko-KR, zh-CN, zh-TW, th-TH, tr-TR,
ru-RU, id-ID, es-419, es-ES. We strip POSIX encoding/modifier suffixes first;
C/POSIX falls back to en-US. The shop receives the mapped locale's first two letters.
Products have
`id`, `name`, `latestVersion`, `productTypeSummary.stickerSummary.stickerIdRanges[{start,size}]`
(or `sticonSummary`). Package metadata:
`https://stickershop.line-scdn.net/stickershop/v1/product/<pkg>/android/productInfo.meta`;
images: `…/stickershop/v1/sticker/<id>/android/sticker.png`, emoji
`…/sticonshop/v1/sticon/<product>/android/<nnn>.png` (`_animation.png` variant).

## Image and file sending (P3)

Image, E2EE (flow observed in order):
1. `TalkService/acquireEncryptedAccessToken` `[2]` → string (1287 chars): token for obs.
2. `TalkService/negotiateE2EEPublicKey` `["<mid>"]` → `{allowedTypes:[…], publicKey:{version,keyId,keyData,createdTime}, specVersion:2}`.
3. `TalkService/determineMediaMessageFlow` `[{"chatMid"}]` → `{flowMap:{"1":2,"2":2,"3":2,"14":2}, cacheTtlMillis}` (content type → flow; 2 = E2EE upload).
4. Two `POST https://obs.line-apps.com/r/talk/emi/<oid>` → 201, binary body (encrypted original, then preview),
   `content-type: application/x-www-form-urlencoded`, header `x-obs-params`: base64 JSON `{"ver":"2.0","name":"<digits:13>|<str:23>","type":"<str:4>"}`, plus `x-line-access`.
5. `TalkService/sendMessage` `[<reqSeq>, {…,"contentType":1,"contentMetadata":{"MEDIA_CONTENT_INFO":"{category,fileSize,extension,animated,width,height}","MEDIA_THUMB_INFO":"{width,height}","SID":"emi","OID":"<str:62>","FILE_SIZE":"<digits>","e2eeVersion":"2"},"hasContent":true,"chunks":[5 Letter Sealing chunks]}]`.
   Echo: operation 25, then 43.

File: `POST https://obs.line-apps.com/r/talk/emf/<oid>` (201), then `sendMessage`
with `contentType` 14, `contentMetadata {"FILE_SIZE","SID":"emf","OID","e2eeVersion":"2"}`
and chunks; the file name is not in plaintext metadata (inside the encrypted
payload, inferred). Echo carries `FILE_EXPIRE_TIMESTAMP`. Files whose extension
is not in the server configuration `function.file.extension.whitelist`
(`getConfigurations`) are wrapped by Chrome in an uncompressed `<name>.zip` after
a "Send compressed" dialog whose "always compress" checkbox defaults to on
(`main.js`, `compressFile`).

## Profiles and contacts (P6, U)

- Cover: `GET …/api/timeline/getCover?targetMid=<mid>&myMid=<mid>&myRegion=JP`
  → `{profileId, coverObsInfo:{obsNamespace:"c", serviceName, objectId}, isDefaultCover}`;
  image `GET https://obs.line-apps.com/r/myhome/c/<objectId>`.
- `GET …/api/timeline/homeId?eMid=<mid>` → string.
- `RelationService/getTargetProfileNotice` `[{"targetUserMid"}]` → `{notice:{noticeAntiFraudGeneral:{}}}` (non-friend fraud warning; also `noticeAntiFraudDifferentRegion`).
- `ChannelService/issueChannelToken` (profile open) → `{token, obsToken, expiration, refreshToken, channelAccessToken}`.
- `BuddyService/getBuddyDetail` `["<mid>"]`: official accounts → `{memberCount, addable, acceptableContentTypes, iconType, botType, …}`; users → 400 with `code 9` (Chrome calls it for every profile and ignores the error).
- Add friend: `RelationService/addFriendByMid` `[{"reqSeq","userMid","tracking":{"reference":"","trackingMetaV2":{"chat":{"chatMid"}}}}]` → `{}`; operation 4.
- Rename (override display name): `TalkService/updateContactSetting` `[<reqSeq>, "<mid>", 2, "<new name>"]` → contact gains `displayNameOverridden`, `settings:"2"`; operation 49.
- Contacts: `getContactsV2` `[{"targetUserMids":[…]}, 2|3]`; contact fields `type`, `status`, `relation` (0 friend, 2 non-friend observed), `displayName`, `displayNameOverridden`, `statusMessage`, `picturePath`, `pictureStatus`, `favoriteTime`, `profileId`, `settings`.
- Blocked lists: `getBlockedContactIds [2]`, `getBlockedRecommendationIds [2]`.

Non-friends cannot be messaged from a profile in Chrome: the card offers
Report, Block and Add friend only, with the fraud notice.

## Groups (P5)

- Mute (notification toggle) and other chat attributes: `TalkService/updateChat`
  `[{"reqSeq","chat":{full chat object incl. notificationDisabled, chatName, picturePath, extra.groupExtra}, …}]` → `{}`;
  operation 121 plus a system message (operation 25, `contentType` 18).
- Leave: `TalkService/deleteSelfFromChat` `[{"reqSeq","chatMid"}]` → `{}`; operations 127, 41.
- `getChats` `[{"chatMids":[…],"withMembers":true,"withInvitees":true},1]` → `extra.groupExtra {creator, memberMids:{mid:joinTime}, inviteeMids, preventedJoinByTicket, invitationTicket, addFriendDisabled, ticketDisabled, autoName}`.

## From `main.js` source (not captured)

Request shapes read from the extension's call sites. Response handling is
inferred.

- React: `TalkService/react` `[{"reqSeq","messageId","reactionType":{"predefinedReactionType":N}}]`, with N one of 2 NICE, 3 LOVE, 4 FUN, 5 AMAZING, 6 SAD, 7 OMG. Picking the current reaction again cancels it: `TalkService/cancelReaction` `[{"reqSeq","messageId"}]`.
- Update a chat: `TalkService/updateChat` `[{"reqSeq","chat":{…full chat with the one field changed},"updatedAttribute":N}]`, where N is one of 1 NAME, 2 PICTURE_STATUS, 4 PREVENTED_JOIN_BY_TICKET, 8 NOTIFICATION_SETTING (`notificationDisabled`), 16 INVITATION_TICKET, 32 FAVORITE_TIMESTAMP (`favoriteTimestamp` as a string, "0" to clear), 64 CHAT_TYPE.
- Create a group or room: `TalkService/createChat` `[{"reqSeq","type":0 GROUP|1 ROOM,"name":"<trimmed or empty>","targetUserMids":[…without self]}]` → `{chat:{chatMid,…}}`. A 1:1 chat with a friend needs no call: the first `sendMessage` to the friend's MID creates it.

## Media upload, E2EE V2 (from `main.js` source)

This complements the captured flow in "Image and file sending (P3)". The
pieces the capture redacted come from the extension's upload code.

1. The message's `contentMetadata.ENC_KM` holds the key material: random, base64-encoded 32 bytes. Both the file and its preview are encrypted with it, using the same scheme received media are decrypted with (`assets.rs`). It travels inside the sealed chunks, never in plaintext metadata.
2. The client picks a temporary object id `reqid-<uuid>` and uploads the encrypted original:
   - `POST https://obs.line-apps.com/r/talk/<sid>/reqid-<uuid>`, where `sid` is `emi` for an image, `emv` video, `ema` audio, `emf` file;
   - the body is the ciphertext;
   - header `X-Obs-Params: base64(JSON {"ver":"2.0","name":"<Date.now() digits>","type":"file"})`. In V2, `type` is always "file" and `cat` is absent;
   - plus the auth headers.
3. The response header **`x-obs-oid`** carries the real object id. That becomes `OID`.
4. Image or video preview: `POST …/r/talk/<sid>/<oid>__ud-preview`, with the encrypted preview as the body and `X-Obs-Params` `name` set to the preview file's name. Video also uploads chunk hashes to `<oid>__ud-hash`, which is not needed for images or files.
   Photos at original quality (what this client sends: no re-encoding) carry `MEDIA_CONTENT_INFO` = JSON `{"category":"original","fileSize":<bytes>,"extension":"<jpg|png|gif|webp…>","animated":<bool>,"width":W,"height":H}` and `MEDIA_THUMB_INFO` = JSON `{"width","height"}` of the preview. The image also gets `ENC_KM` if it has none. Chrome's default quality re-compresses the image and omits `MEDIA_CONTENT_INFO`. Images over 1e8 pixels are sent as files.
5. Then `sendMessage` with `contentMetadata` `SID`, `OID`, `FILE_NAME` and `FILE_SIZE` (files and images), the media info (`MEDIA_CONTENT_INFO`, `MEDIA_THUMB_INFO` for images) and `e2eeVersion` "2", sealed like a text message. Progress is local only.

## Unsend

`TalkService/unsendMessage` `[<reqSeq>, "<server message id>"]` → `{}` (observed
for two own file messages). Live: operation 64 DESTROY_MESSAGE (`param1` chat,
`param2` message id) on the sender's devices; other members receive 65
NOTIFIED_DESTROY_MESSAGE (inferred from the enum). Chrome replaces the message
with a centred system pill "Message unsent." rather than a bubble.

## Startup sequence (reference for D4/D5)

`getProfile`, `getEncryptedIdentityV3 []` (→ `wrappedNonce`, KDF parameters for
local storage), `getServerTime`, `getLastOpRevision`, `getConfigurations`,
`getSettings [2]`, `getE2EEPublicKeysEx [false,1]`, shop summaries, then
`GET /api/operation/receive?localRev=…&version=3.7.2&lastPartialFullSyncs=<json>&language=…`
in parallel with `getAllContactIds`, `getAllChatMids`, `getMessageBoxes`,
`getContactsV2` batches of 100, `getLastE2EEPublicKeys ["<mid>"]`,
`getE2EEGroupSharedKey [1,"<chat>",<keyId>]`, `getMessageBoxesByIds`,
`getChats`. Opening a chat: `getRecentMessagesV2 ["<chat>",50]`,
`getMessageReadRange`; link previews via
`GET https://legy-jp.line-apps.com/sc/api/v2/pageinfo/get?url=…&caller=LINE_CHROME`.

## Reference screenshots (U0)

Local only. Friend profile (cover band, 96 px-class circular avatar overlapping
it, name with rename pencil, status line, favourite star, bottom bar with a
single chat action), non-friend profile (Report button, fraud notice, bottom bar
Block and Add friend, no chat), group info popup (photo with camera badge, name
with pencil, member search, Invite friends, collapsible "Members N" with remove
×, "Members join automatically" setting), invite picker (Recent chats and
Friends N with checkboxes, selection pane, Invite). Profiles open in LINE's
separate popup window; the main window keeps the chat.

## Chat events (extension source, 2026-10-19)

`main.js` defines `CHATEVENT = 18`. `contentMetadata.LOC_KEY` selects
an event; `LOC_ARGS` splits on record separator U+001E into argument groups,
then on unit separator U+001F into values. These are **not JSON arrays**.
The renderer resolves user IDs in the arguments through contacts, rather
than treating `text` as the event description. Unknown/empty keys render
nothing (the extension also blanks keys when removing an event).

| LOC_KEY | Argument groups | Meaning |
| --- | --- | --- |
| C_PN | actor MID; new name | Group renamed |
| C_PI | actor MID | Group photo changed |
| C_MJ / A_MJ | member MID | Member joined |
| C_ML / A_ML | member MID | Member left |
| C_GI / C_MI / A_MC / A_MI | actor MID; invitee MIDs | Invited/added members |
| C_MR / A_MR | actor MID; member MID | Member removed |
| C_IC / A_IC | actor MID; invitee MID | Invitation canceled |
| C_SP / C_SN | actor MID | Joining via link/QR allowed / disabled (as mapped by extension) |
| C_BG | none | Chat upgraded to group |
| pseudo_leave_chat | literal display name | Locally synthesized departure |
| BB | old album name; new album name | Album renamed (actor is message.from) |
| BD / BO | album name | Album deleted / photos removed (actor is message.from) |
| C_OL | actor; invitee MIDs | Invitees at chat limit |

English resources in the English JS locale chunk use
`groupchat.systemmsg.changegroupname` (“%1 changed the group's name to
\"%2\".”), `userjoin` (“%1 joined the group.”), `userleave`, `inviteuser`,
`kickoutuser`, `cancelinvite`, and `changegroupphoto`. Auto-group variants
also offer a group-name update link. The app uses concise single-line
sentences. Synthetic example:
`{"contentType":18,"contentMetadata":{"LOC_KEY":"C_PN","LOC_ARGS":"aoi\u001eWeekend plans"}}`.

Unread count is message-box metadata, independent of history projection.
Recent/older decoding retains unsupported types as `Content::Unknown`.
Read exposure previously required a fully visible rendered row, so an empty
or omitted tail had no marker target even with a server newest-ID hint.
At the loaded history bottom the app now covers that hint; unknown event
rows remain stored and unsupported content gets an explicit bubble.
The offline “Official hidden events” chat reproduces two unread events
with no visible message rows.

## Replies, checked markers, reactions, and profile status (2026-10-19)

Source inspection: installed LINE Chrome 3.7.2 `static/js/main.js`, offline;
these are extension call shapes, not a live capture.

- `SD` posts positional arguments to `/api/talk/thrift/Talk/TalkService/<method>`.
  The shared Axios success interceptor tests `response.data.message === "OK"`
  and returns `response.data.data`. It does **not** require `code` or `data`.
  A successful void result may therefore omit `data` entirely.
- `sendChatChecked`: `[reqSeq, chatMid, messageId, 0]`. The read observer calls
  `ne.mutate([HO(), e, r.id, 0])`; no result payload is consumed.
- `react`: `[{reqSeq, messageId, reactionType: {predefinedReactionType}}]`;
  `cancelReaction`: `[{reqSeq, messageId}]`. The reaction menu uses these
  mutations for types 2–7; it consumes no success payload.
- Our RPC decoder parses JSON, then accepts HTTP 200 with `message: "OK"`.
  Absent data becomes Null. Every other envelope returns an RPC error with
  its code (or zero when absent). Decode diagnostics contain
  HTTP status and byte length; never response content or ids.
- Composer text and sticker enqueue calls pass a `relation` with
  `relatedMessageId: replyTargetMessageId`, `messageRelationType: 3`
  (`wU.REPLY`), `relatedMessageServiceCode: 1` (`yU.TALK`). The send queue
  merges these fields onto the `sendMessage` message object, outside
  `contentMetadata` and outside the encrypted text payload. They survive retries.
  The attachment enqueue callback receives only the files and no relation;
  attachments therefore follow the extension's behavior without a reply.
- Contacts/profile status use `statusMessage` with
  `statusMessageContentMetadata`. The extension passes both to `s1`, the same
  rich text renderer used for messages. `REPLACE.sticon.resources` has the same
  UTF-16 ranges and emoji resource fields as message content metadata. Without
  this metadata, parenthesized codes remain literal text. Status metadata is
  persisted with contacts and rendered through the existing emoji preview path.

## Logout (2026-10-20; from main.js source, not captured)

Read only the installed LINE Chrome 3.7.2 manifest and `static/js/main.js`.
The path table defines `sj = SD("talk/thrift/Talk/AuthService/logoutV2")`.
The logout handler `xT` awaits `sj({ignoreTalkAuthException: true})` and
performs local cleanup in `finally`. `SD(e,t)` returns `(n={}, ...r)`;
`n` is Axios configuration and `r` is the JSON request body. Therefore:

- POST `/api/talk/thrift/Talk/AuthService/logoutV2`
- Service: `AuthService` (under `Talk`)
- Arguments: `[]`; `ignoreTalkAuthException` is not a wire argument.

Sen uses the existing authenticated/signed request, with a three-second
whole-request timeout and no refresh/retry. Server errors are logged and do
not prevent local logout. Loopback tests verify the path and argument array;
the real call remains live unverified.
