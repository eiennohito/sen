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
| 25 | Opening a chat from a notification or D-Bus activation skips saving the current draft (6 copies of "open chat") | audit | deslop pass, desktop glue batch |
| 26 | Deslop batches from the audit: store schema/unread model, transport rewrite, delete CLI and native_crypto, credentials data-loss paths, client core, engine split, content model, desktop glue, check-ui restructure (15 launches → 2), test deletions | audit | wave 1 merged (crypto/CLI, store, transport); wave 2 merged (backend/demo, client core, content model); wave 3: engine 7a dedup merged (behaviour-changing items moved to #32), check-ui restructure in review; reconnect-retry and the asset priority mailbox are deleted, not wired (desktop glue batch) |
| 32 | Engine behaviour cleanups that need a design, not a dedup pass: the 100 ms health poll (wait until the next deadline without missed wakeups), raw live-op numbers in the engine (typed ops; empty `param1` must still checkpoint), nested catch_unwind boundaries (one boundary; panic payload reported; no abort on a poisoned lock), silent drops (clipboard prune, sticker/photo decode, start_assets failure), startup probe mapping every error to SessionUnusable, `NETWORK_TOUCHED` thread-local. Constraints learned: uploads stay off the net thread (a running upload must never delay sends, reads, refresh or the stream); queued intents (final draft) run before stop; cancelled RPCs still complete their UI state | engine 7a attempt (discarded) | after the narrow 7a dedup; one item per batch, each with a test that would have caught the regression |
| 30 | UI check launch race: a group occasionally connects before the app listens (`Connection refused` at 0.2 s) | found | wave 3 check-ui restructure: wait for the server's "listening" line |

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
- Deslop wave 1, transport: hyper on a small async runtime instead of ureq plus a custom DNS resolver and polling connector (−1.4k lines). Cancel drops the request; one idle connection per host; only provably unsent requests are resent; HTTPS only; env proxies fail closed; upload deadline is idle-based with real progress.
- #24 Demo and the fake backend compile only with the `demo` feature (ui-debug implies it); the product binary has no demo code or flags. Backend/controller/reporting deslop; reporting logs real SQLite/OS codes. #29 one random helper.
- Client core: LINE codes matched only in error.rs; typed saved session in the exact on-disk format (round-trips unknown fields); distinct saved-session failures (a locked keyring asks to unlock); QR drawn directly. #27 ServerId orders numerically. #31 crypto tests use the installed extension, no pinned fixture. Store schema 10 (local store discarded once).
- Content model: one kind label table (compact for previews/notifications/reply bar, full for rows), grouping over typed messages, core contact/reply-target/profile rules, per-entity name fallbacks. #28 computed unread lives on the list item. Store schema 11.
- #18 UI check `focus` 46 s → 11 s (95 → 14 s wide); all 16 groups ~80 s.
