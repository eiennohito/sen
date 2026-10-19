# Issues

One list of everything reported or found, with its state. Updated on every
merge. "Branch" means the work exists but is not merged yet.

## Open

| # | Issue | Source | State |
|---|---|---|---|
| 1 | Read markers (`sendChatChecked`) and reactions (`react`) fail on the real server: "response decode failed" | user log | in progress, branch `review2` |
| 4 | Unread count with no visible messages (LINE official account) | user | fixed in branch `review2`; on the server it also needs #1 |
| 5 | React button shows/hides under unclear conditions | user | fixed in branch `review2` |
| 6 | Circular avatars: uneven halo on the edge in dark theme | user | fixed in branch `review2` |
| 7 | System events shown as "(i) System message", icon misaligned | user | fixed in branch `review2` |
| 8 | Reply quotes displayed incorrectly | user | in progress, branch `review2` |
| 9 | Cannot reply (quote) when composing | user | in progress, branch `review2` |
| 10 | Message run grouping looks bad (rated 1.5/10) | user | in progress, branch `review2` (spec in visual-design.md) |
| 11 | Friend status: "(xxx)" emoji wrong, text wraps vertically | user | in progress, branch `review2` |
| 12 | Long text overflows list items | user | in progress, branch `review2` |
| 15 | God objects: `State` (213 properties), `AppWindow` with 85 forwarding aliases | user | R4, not started |
| 16 | Generated Slint Rust is 10 MB | user | R4: interpreter in dev builds, compiled releases cut by source rules |
| 17 | Global deslop pass, including a whole-suite test audit | user | after `netmodel2` and `review2` merge |
| 19 | History decrypt: newest first, show messages as they decrypt; measure a compiling WASM engine | user | after the deslop pass |
| 20 | Corrupt message whose auth tag fails poisons the WASM runtime (~150 ms per channel to rebuild) | found | accepted for now; revisit with #19 |
| 21 | HANDOFF.md describes the old architecture | found | after #15–#17 |
| 22 | Untraced actions: block, report, invite, accept/reject invitations, remove member | found | not started |
| 23 | Live account verification | constraint | needs the device slot (user's call) |

## Done (merged to main)

- Sign-in showed no QR; Cancel did nothing (backend open failed silently, net thread died).
- Settings icon garbled; checkboxes misaligned; Friends empty state said "No conversations yet"; empty profile avatar while signed out.
- "Local data could not be saved" for unrelated failures; typed, specific error reporting.
- App ignored `paths.json`, then `paths.json` itself removed; compatibility code removed; PoC binary removed; crate renamed `line`.
- Crypto module follows the installed Chrome extension; checked by typed interface, not hash.
- One WASM runtime; send channels and signing key cached (send 142→5 ms, signing 74→10 ms).
- Attachments (files, photos, clipboard) with uploads off the network thread.
- Library tests 40 s → 2 s (dependencies optimized in dev builds).
- #2 Failed read markers retried forever / log spam: Bug-class failures are not retried; causes are logged once per operation.
- #3 Rejected session stuck "Connecting": LINE's in-body codes 1/7/8 sign out with a reason; 119 refreshes once; persistent failures become visible.
- #13 Resend across sign-out/sign-in: nothing resends across sessions or restarts; failed sends are "Not sent" with Retry; our echo turns any row into Sent.
- #14 Transient drops invisible, "No connection" only after 15 s without a network success, "Signed out: <reason>" as one state; the stream restarts after drops; undecryptable messages no longer break loading.
- #18 UI check `focus` 46 s → 11 s (95 → 14 s wide); all 16 groups ~80 s.
