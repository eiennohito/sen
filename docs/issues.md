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
| 24 | Demo scenes and fixtures are compiled into the product binary | found | deslop pass: move to a dev package behind a feature |
| 25 | Opening a chat from a notification or D-Bus activation skips saving the current draft (6 copies of "open chat") | audit | deslop pass, desktop glue batch |
| 26 | Deslop batches from the audit: store schema/unread model, transport rewrite, delete CLI and native_crypto, credentials data-loss paths, client core, engine split, content model, desktop glue, check-ui restructure (15 launches → 2), test deletions | audit | wave 1: crypto/CLI and store merged; transport being rewritten on hyper + smol-family runtime (chosen over reqwest+tokio and isahc: 2.4 MB vs 5.7/6.3 MB, 52 vs 92/113 crates) |
| 27 | Server id ordering exists in four forms: `server_sort` padding, the same padding in SQL, `(len, str)` comparisons in reading.rs, and a derived `Ord` on `ServerId` that compares strings (wrong numerically) | store review | wave 2 client core: numeric `Ord` on `ServerId`, one sort key |
| 28 | `ChatInput` carries both the raw server count and the computed `unread_count`; the demo set the wrong one and only a UI check caught it | store review | wave 2 content model: computed count lives on the list row, not the input |
| 29 | Three private `/dev/urandom` readers (runtime, credentials, engine reqSeq) | store review | wave 2 client core: one helper |
| 30 | UI check launch race: a group occasionally connects before the app listens (`Connection refused` at 0.2 s) | found | wave 3 check-ui restructure |

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
- Deslop wave 1, crypto/CLI: `native_crypto` and the `line-auth` CLI deleted (−2.2k lines); an unreadable credentials file or failed keyring unlock is an error, never a fresh key over the sealed session.
- Deslop wave 1, store: one schema with a version check (a change discards the local store), validated `ServerId`, one unread function (reading to the newest message clears the count at once), reqSeq random per process like the extension.
- #18 UI check `focus` 46 s → 11 s (95 → 14 s wide); all 16 groups ~80 s.
