# Issues

One list of everything reported or found, with its state. Updated on every
merge. "Branch" means the work exists but is not merged yet.

## Open

| # | Issue | Source | State |
|---|---|---|---|
| 1 | Read markers (`sendChatChecked`) and reactions (`react`) failed on the real server: "response decode failed" | user log | merged; success is now the extension's `message == "OK"` rule. Needs confirmation in the real app |
| 4 | Unread count with no visible messages (LINE official account) | user | merged; the read marker covers hidden events. Needs confirmation in the real app (depends on #1) |
| 15 | God objects: `State` (213 properties), `AppWindow` with 85 forwarding aliases | user | R4, not started |
| 16 | Generated Slint Rust is 10 MB | user | R4: interpreter in dev builds, compiled releases cut by source rules |
| 17 | Global deslop pass, including a whole-suite test audit | user | next |
| 19 | History decrypt: newest first, show messages as they decrypt; measure a compiling WASM engine | user | after the deslop pass |
| 20 | Corrupt message whose auth tag fails poisons the WASM runtime (~150 ms per channel to rebuild) | found | accepted for now; revisit with #19 |
| 21 | HANDOFF.md describes the old architecture | found | after #15–#17 |
| 22 | Untraced actions: block, report, invite, accept/reject invitations, remove member | found | not started |
| 23 | Live account verification | constraint | needs the device slot (user's call) |
| 24 | Demo scenes and fixtures are compiled into the product binary | found | in the deslop pass (#17) |

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
- #5 React button: one visibility rule (pointer over the row, or the message selected/focused), own slot.
- #6 Avatar edge: round bitmap rendered at the displayed size, no second aliased clip.
- #7 System events render what happened ("Aoi renamed the group to …"); unknown events are hidden.
- #8 Reply quotes: tinted bar, readable name, media label and thumbnail, "Original message" when missing, click jumps to the original (fetched if needed).
- #9 Replying: menu and hover action, composer reply bar, kept in the draft, sent with the extension's relation fields (text and stickers).
- #10 Message runs: separate bubbles, 2/12 px spacing, small corners on the sender side, time once per run.
- #11 Friend status emoji from `statusMessageContentMetadata`, single line.
- #12 List rows: single-line text from the presenter, one shared row component.
- #13 Resend across sign-out/sign-in: nothing resends across sessions or restarts; failed sends are "Not sent" with Retry; our echo turns any row into Sent.
- #14 Transient drops invisible, "No connection" only after 15 s without a network success, "Signed out: <reason>" as one state; the stream restarts after drops; undecryptable messages no longer break loading.
- #18 UI check `focus` 46 s → 11 s (95 → 14 s wide); all 16 groups ~80 s.
