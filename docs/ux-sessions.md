# Open problems

Written 2026-10-17 from an audit of commit `05cd26d`: headless snapshots at
980×720, 1400×900, 760×600 and 360×520 (15 and 24 px), a driven run on a
private virtual KWin through the Slint MCP server, and source reading. The
behaviour to build is still [ux-model.md](ux-model.md). The previous session
plan and its hand-offs are in the git history of this file; they described
work as done that this list shows is not.

Rules that still apply: Slint 1.18.1 pinned, software renderer, no async
runtime, worker threads own network and crypto, UI thread owns models, no
account access without the device slot. Agent-driven previews go through
`tools/run-ui-debug.py`.

Order of work: T first, because every "slow" item below it is partly T. Then
P, then the UI groups in any order. V closes the loop.

## T — Transport and worker

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| T1 | Every RPC spawns a `curl` child, pipes the token on stdin, drains stdout on a second thread, and polls `try_wait` every 20 ms. No connection reuse; one TLS handshake per request. | `src/client.rs:154`, `:211` | One persistent blocking HTTP client (`reqwest::blocking` with rustls, already in the cargo cache) owned by the worker. Keep the host allowlist, size caps and no-redirect rules. |
| T2 | Every image (avatar, sticker, preview, reaction asset, file) is its own `curl` child, fetched serially on the single worker thread between commands. Queue capped at 256, processed newest-first. | `src/client/desktop/assets.rs:211`, `app/src/account.rs:499` | Two to four blocking fetcher threads with a shared client; prioritise assets for the visible range; keep the worker free for Send and Messages. |
| T3 | SSE stream is `curl` stdout split into lines by a reader thread and polled every 100 ms; headers detected by string match; stream death noticed only by the 30 s ping watchdog. | `src/client/desktop/sync.rs:370`, `:413` | Blocking response reader on the existing sync thread, `read_line` with the stop flag checked per line. |
| T4 | Image failures discard the error (`Err(_)`), blacklist the key for 300 s, and the placeholder says "Refresh to retry" while Refresh re-publishes the cached failure without a network call. | `app/src/account.rs:499`, `:516`; `app/ui/message.slint:149` | Keep the failure class (timeout, HTTP status, unsupported host, rejected OID). Retry on explicit user action. Placeholder wording must match what Retry will actually do. |
| T5 | Reaction counts never update live: the sync decoder handles no reaction operation; counts change only when a message page is refetched. | `src/client/desktop/sync.rs` (no match for reaction) | Decode the reaction operation into an in-place row update; it is a quiet event in the attention matrix. |
| T6 | Every reconnect (normal SSE end, ping expiry) is reported as `Reconnecting` for five attempts and the view maps that to `connection_failed`, raising the 116 px auth strip and shifting the whole shell. | `src/client/desktop/sync.rs:216`, `app/src/view.rs:553`, `app/ui/app.slint:148` | Reconnecting is a quiet header state plus a transient "Reconnected". The strip with Retry and Sign in again appears only for Offline and AuthExpired. The strip must overlay, not push. |
| T7 | Sign-in shells out to `qrencode`, and fails if it is missing. The CLI still carries kitty terminal-graphics output. | `src/client/pairing.rs:81`, `src/client.rs:411`, `:445`, `:455` | Pure-Rust QR encoding; drop the terminal rendering from the library. |
| T8 | `xdg-open` is spawned unguarded and never reaped. | `app/src/links.rs:155` | Reap on a helper thread. |
| T9 | Crypto runs the Chrome WASM blob in the `wasmi` interpreter for every encrypt and decrypt; the sync thread instantiates a second runtime; the user must supply `ltsm.wasm`. | `src/runtime.rs:506` | Research item, not a session: document the key schedule, port the primitives (`aes`, `ctr`, `hkdf`, `hmac`, `sha2` are already dependencies), keep wasmi as a test oracle. |
| T10 | Library default state path is `artifacts/auth/session.json` inside the repository; the installer's `--dev-paths` writes it into the desktop entry. | `src/client.rs:43`, `tools/install-desktop.py:55` | Library defaults follow XDG like the app does. |
| T11 | Session credentials (refresh token, wrapped keys) are plaintext JSON at mode 0600; drafts are encrypted, credentials are not. | `src/client.rs:88`, usage text at `:771` | Same encrypted store as drafts, or the desktop secret service. |

## P — Protocol gaps

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| P1 | Official-account media uses a different route. The client only builds `obs.line-apps.com/r/talk/{SID}/{OID}/preview` with defaults `m` and the message id, so OA images 404 and show as "Preview unavailable". The map records that `DOWNLOAD_URL` variants are refused on purpose. No OA message exists in `dumps/`. | `src/client/desktop/assets.rs:106`, `docs/chromium-ui-map.md:351` | Trace one OA image message (its `contentMetadata` keys and resolved host), add that host to the allowlist, build the right URL. Until then say "Not available for this sender", not "Refresh to retry". |
| P2 | Sticker sending does not exist: no picker, no send path. Listed as "later". For a LINE client it is a daily-use blocker. | `app/ui/composer.slint`, `app/src/composer.rs` | Trace one sticker send (STKID/STKPKGID/STKVER shape is in the HAR), implement the send, then a picker as a panel in wide and a sheet in narrow. |
| P3 | Attachment sending, paste and drop of media do not exist (CMP8 media, CMP9, OS10). | composer | After P2; needs the upload trace. |
| P4 | Link detection only matches `https://`, `http://`, `www.`. Bare domains, uppercase schemes and links split across emoji runs are missed. | `app/src/links.rs:13` | Broaden the matcher; run it over the rendered text, not the pre-run body. |

## S — Shell and layout

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| S1 | Profile panel hides the whole browser pane below 1024 px, including at the default 980 px window. Clicking a friend removes the list you clicked in. The panel also paints over the connection strip. | `app/ui/app.slint:22`, `app/ui/layers.slint:65` | Panel narrows the conversation only; below some width it becomes a sheet (model §3.1). Panel sits below the strip. |
| S2 | Profile panel is not bound to a destination or chat: switching to Chats, Alt+Up/Down, or any navigation leaves a stranger's profile over the conversation with the chat list hidden. | `app/src/browser.rs:4` (`browse` never closes it) | Close the panel on destination switch; close on chat switch unless it is that chat's profile. |
| S3 | Open chat from a profile leaves `section` on Friends: Chats rail not selected, Friends list has no selection, and a friend with no history opens a blank white pane with no empty state. It also appends a synthetic chat row with no time or preview. | `app/src/browser.rs:103` | Switch to Chats, select the row, show "No messages yet" in the history. |
| S4 | Rail "Account" button opens Settings on the Chats section; it is a duplicate of the Settings button in both wide rail and narrow bar. | `app/ui/rail.slint:26` | Open About, or remove the button. |
| S5 | Chat-row hover is invisible: hover colour equals the pane background. | `app/ui/chats.slint:83` vs `app/ui/app.slint:172` | Distinct hover role. |
| S6 | Settings dialog: Close button above the title, a ComboBox as section navigation, roughly 80 % empty space at 560×640. | `app/ui/layers.slint:93` | Title first, sections as a left list or tabs, size to content, Close at the bottom or as the title-row icon. |
| S7 | Connection strip is a fixed 116 px block with two full-width buttons that pushes the shell down. | `app/ui/app.slint:146` | One-line strip, inline actions, overlay, no layout shift. |
| S8 | History focus ring: a 2 px accent border around the entire history pane on any click into empty history space, after closing the viewer, after Resend or Cancel. It conveys nothing. | `app/ui/conversation.slint:68`, `app/ui/scrollpane.slint:15`, `app/src/composer.rs:185`, `app/ui/app.slint:90` | Remove the pane ring. Focus rings mean "you are typing here" (composer) and "this message is selected" (bubble), nothing else. |
| S9 | Bubble focus border uses the same accent ring as the composer. | `app/ui/message.slint:62` | Style as a selection highlight, not a focus ring. |
| S10 | Composer is a blank bordered box with no placeholder and a 132 px minimum height; at 360×520 with 24 px text the reason caption wraps to two lines and the field is one line tall. | `app/ui/composer.slint:9`, `app/ui/draft-editor.slint` | Placeholder "Message", minimum of two lines, caption only when there is a reason. |
| S11 | "Latest" and "Return to message" are full-width standard buttons between history and composer that change the history height when they appear. | `app/ui/conversation.slint:85`, `:98` | Floating pill over the history bottom edge. |
| S12 | Friends header rows use text glyphs "▾/▸" (also inside accessible labels); both headers render even when a section is empty. | `app/ui/friends.slint:73`, `app/src/browser.rs:58` | Icon, and omit empty sections. |
| S13 | Friend avatar fallback shows the elided full name ("新…") instead of an initial. | `app/ui/friends.slint:76` | Use the initial like chat rows do. |
| S14 | Narrow bottom bar has four slots for three destinations (see S4). | `app/ui/rail.slint:40` | Follows S4. |

## M — Messages

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| M1 | URLs render twice: plain in the body (not styled, not clickable) and again as a centred underlined run under the body that looks like a preview card. | `app/ui/message.slint:113` | Until inline styling exists, render the body once with the URL run replacing the matching text, not duplicating it. |
| M2 | Unbroken URLs paint past the bubble: body `TextInput` and link `Text` both use `word-wrap` with no character-break fallback; bubble width is capped at 480. | `app/ui/message.slint:93`, `:120` | `char-wrap` fallback for runs longer than the bubble. |
| M3 | Single click on an emoji message replaces it with fallback text and injects a "Show emoji" button into the bubble. | `app/ui/message.slint:81` | Selection mode only from the context menu; a click focuses, nothing more. |
| M4 | Reply quote carries a full-width button labelled "Reply" that jumps to the quoted message. | `app/ui/message.slint:68` | Clickable quote block, no button, no "Reply" label. |
| M5 | Rich text layout leaves a gap after the first emoji run on a line (stale run measurement). | `app/ui/rich-flow.slint`, `app/src/rich_content.rs` | Re-measure after the first layout pass before positioning. |
| M6 | Failed-send rows put the delivery mark beside the Retry/Edit buttons, not beside the bubble, and drop the time. Retry/Edit are 160 px standard buttons. | `app/ui/message.slint:153`, `:181` | Mark stays in the time slot beside the bubble; actions are compact text buttons under the explanation. |
| M7 | File and media rows are ordered Save, size, name; duration above "Video". | `app/ui/message.slint:70`, `:72` | Name, then size or duration, then action. |
| M8 | Text selection inside an outgoing bubble is invisible: `outgoing-bubble` and `selection` are the same palette role. In dark themes incoming bubbles are close to it as well. | `app/ui/tokens.slint:22`, `:25` | Own role for the outgoing bubble; contrast-check selection against both bubbles. |
| M9 | Unread badge on the selected chat row is accent on selection (blue on blue). | `app/ui/chats.slint:98` | Invert on selected rows. |

## C — Composer, input, feedback

| ID | Problem | Where | Fix |
| --- | --- | --- | --- |
| C1 | Ctrl+Enter does not send while the non-modal profile panel is open; it inserts a newline. | `app/ui/draft-editor.slint:56` | Only modal layers block it (`State.modal-open`). |
| C2 | Chat-list load failure flips `connection_failed` and raises the auth strip with "Sign in again"; the footer text refers to a "More conversations" button that is labelled "Load more". | `app/src/view.rs:1032`, `:1036` | List failures stay in the list footer. Fix the wording. |
| C3 | Implementation vocabulary in production strings: "Unread boundary unavailable", "Large message retained; some previews may be unavailable", "Preview unavailable · Refresh to retry", "No conversations loaded", "Reconnecting… (attempt N)". | `app/ui/conversation.slint:24`, `:97`; `app/ui/message.slint:149`; `app/ui/chats.slint:61`; `app/src/view.rs:539` | Rewrite per model §9.2. |
| C4 | Three error strings tell the user to "Use Refresh"; the Refresh buttons are leftovers from before live sync and are the only image-retry path. The refresh icon is drawn with the arrowheads on the wrong ends. | `app/src/view.rs:765`, `:1036`; `app/ui/chats.slint:23`; `app/assets/icons/refresh.svg` | Remove manual Refresh once T2 and T4 give assets their own retry; fix or replace the icon if any refresh control survives. |
| C5 | Search "Clear" is an always-visible standard button next to the field. | `app/ui/chats.slint:48`, `app/ui/friends.slint:46` | Clear icon inside the field, shown only with a query. |

## V — Verification debt

| ID | Problem | Fix |
| --- | --- | --- |
| V1 | Every inspected snapshot and every MCP run is demo mode, where fifteen scenario buttons crush the chat list to one and a half rows at 980×720 and a 6 px sliver at 760×600. The production list has never been rendered by a reviewer. | Move demo controls into a separate debug sheet or an env-gated panel; snapshot the shell without them. |
| V2 | Demo mode never enables Send; CMP7 is marked verified by MCP, which is impossible. | A demo transport that accepts sends locally, so the real composer path is exercised. |
| V3 | Demo scenarios open scrolled to the top (`show_history(0, false)`); first-open-at-bottom has only synthetic coverage. | Fixture that opens like a real chat. |
| V4 | Demo fixtures: 1970/1/1 date divider, three-line "Synthetic profile" rows, "Synthetic submission" bubbles. The time formatter and the two-line row are not exercised. | Realistic fixtures with current timestamps. |
| V5 | No physical verification at all (Fcitx, scrollbar drag, 100/150/200 % scale, dark theme, notifications, tray, single-instance raise) and no account verification. | A person at the desktop, then assign the device slot. |
| V6 | `ux-acceptance.md` rows marked `yes` over the problems above: VIS3, FB1, FB3, WIN4, IN5, MSG5, RD3 (live), CMP7 (M), OS9, OUT2 (fixed slot), SH8. | Reset those rows when their fix lands; do not pre-emptively edit the history. |
