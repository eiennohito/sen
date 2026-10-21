# Dependencies

The rules for adding crates and why each direct dependency is there. Every
direct dependency adds code, build time and an update surface, so each one
has to earn its place.

## Rules

- **A new direct dependency needs a reason in this file**, added in the
  same change: what it is used for and why the standard library or an
  existing dependency doesn't do it.
- **Prefer what is already in the build graph.** Making an existing
  transitive crate (one Slint, image or zbus already pulls in) a direct
  dependency costs no extra compile time. Pin it to the version already in
  `Cargo.lock`, so it doesn't add a second copy.
- **No new crate for a few lines of code.** If the job is a constant or a
  short loop, write it.
- **Default features off**, enabling only what is used. Crates in the TLS
  and crypto path keep their exact pins.
- **Nothing is embedded that can be loaded.** LINE's crypto WASM and
  reaction artwork come from the installed Chrome extension at runtime. The
  emoji font is installed on the system or downloaded. None of these
  is copied into the repository.
- **Test-only crates go in `[dev-dependencies]`**, never in the product
  dependency list.
- **The UI crate (`ui/`) depends only on Slint.** It is the slow generated
  crate. If it depended on `sen`, every core edit would rebuild it.

## Core (`sen`)

| Crate | Used for |
|---|---|
| anyhow, thiserror | Errors: `anyhow` at the edges, `thiserror` for typed store/HTTP/client errors |
| serde, serde_json | The Talk JSON protocol, the store's JSON columns, preferences |
| wasmi | Runs LINE's unmodified crypto WASM from the installed extension |
| aes, aes-gcm, ctr, hkdf, hmac, sha2 | Media and E2EE crypto around the WASM, sealed session, content hashes in cache file names |
| base64 | Protocol fields (lenient decoding like the extension's `atob`) |
| hyper, http-body-util, bytes | HTTP/1.1 client for Talk, streams and assets |
| async-executor, async-io, async-net, async-channel, futures-lite | A small async runtime for the HTTP client, without tokio |
| futures-rustls, webpki-roots | TLS with Mozilla's root store (ring backend) |
| url, percent-encoding | URL parsing, asset host allow-list, query encoding |
| zbus | Secret Service keyring (session key) |
| rusqlite (bundled) | Local per-account store |
| image | Decoding and resizing photos, avatars and thumbnails into the cache |
| qrcodegen | The sign-in QR code |
| sys-locale | Language of the sticker package list |
| libc | `ENOSPC`/`EROFS` classification of store errors |
| tempfile | Atomic, no-clobber publication of copied/staged files; temporary demo stores |
| ttf-parser | Checking a downloaded emoji font has CBDT/CBLC bitmap tables and emoji glyphs before use (already in the graph through Slint; same version) |
| fontique | The final check that a downloaded font registers (already in the graph through Slint; pinned to Slint's version) |

Dev only: `socket2` and `rcgen`/`rustls` (loopback servers and a TLS test
certificate), `zbus` with `p2p` (private-bus keyring tests).

## Desktop app (`sen-desktop`)

| Crate | Used for |
|---|---|
| slint, i-slint-core | UI: software renderer, winit backend, accessibility; `i-slint-core` for the keyboard-focus hook, window adapter access and font cache invalidation that Slint does not expose publicly |
| i-slint-backend-selector (`ui-debug` only) | The MCP automation server used by `tools/check-ui-mcp.py` |
| sen, sen-ui | Core and the generated UI |
| icu_normalizer | NFKC for Japanese search; already in the graph, pinned to that version |
| unicode-segmentation, unicode-linebreak | Grapheme-safe eliding and line breaking in rich text |
| chrono | Local dates and times in the message list |
| image (png) | Pasted clipboard images (decoding and re-encoding) |
| arboard | Clipboard: pasting images and files |
| zbus | Notifications, tray (StatusNotifierItem), portals, single instance |
| url | File URIs for drag and drop and links |
| serde, serde_json | Settings and demo data |
| libc | `O_NOFOLLOW` when writing exports and autostart entries |
| tempfile (`demo` only) | The throw-away demo store |

Dev only: `i-slint-backend-testing` (headless UI tests).

## UI (`sen-ui`)

`slint`, plus `slint-build` at build time. Nothing else (see the rules).
