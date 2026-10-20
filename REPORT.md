# Chromeless LINE Client - Viability Report

**Date:** 2026-03-17
**Extension analyzed:** LINE Chrome Extension v3.7.2 (`ophjlpahpchlmihnnnihgmmeilfjmjjc`)

## Summary

Building a chromeless LINE client is **viable** by reusing LINE's own WASM blob (`ltsm.wasm`) and speaking JSON to their Chrome-specific gateway. The Chrome extension uses a significantly simpler protocol than the mobile/desktop clients documented in older reverse-engineering efforts.

## Protocol Architecture

### Gateway

The Chrome extension does NOT use the old binary Thrift protocol (`gd2.line.naver.jp`, `/S4`, `/P4`).
Instead, it talks **JSON over HTTPS** to a dedicated gateway:

- **Host:** `line-chrome-gw.line-apps.com`
- **Paths:** `/api/talk/thrift/Talk/TalkService/{method}` (REST-style, despite "thrift" in the URL)
- **Content-Type:** `application/json` (both request and response)
- **Auth:** JWT token in `cookie (lct=...)` and `x-line-access` header

The JWT identifies the client as `ctype: "CHROMEOS"`, `cmode: "SECONDARY"` - it's a secondary device, not a primary client.

### Server Configuration

On startup, the extension hits `ci.line-apps.com/R4?type=Chrome_OS&version=3.7.2&region=JP` to get server endpoints and settings, including:

- `line-chrome-gw.line-apps.com` (Chrome gateway)
- `legy-jp.line-apps.com` (LEGY gateway, used for link previews)
- `obs.line-scdn.net` (media/object storage CDN)
- `profile.line-scdn.net`, `stickershop.line-scdn.net` (static assets)
- `polling_timeout: 140000` (140s long poll)

### Domains Contacted

| Domain | Purpose |
|--------|---------|
| `line-chrome-gw.line-apps.com` | Main API gateway (JSON) |
| `legy-jp.line-apps.com` | Link preview / page info |
| `ci.line-apps.com` | Server configuration |
| `profile.line-scdn.net` | Profile images |
| `stickershop.line-scdn.net` | Stickers |
| `uts-front.line-apps.com` | Telemetry (can likely be skipped) |
| `sentry-uit.line-apps.com` | Error reporting (can be skipped) |

## API Methods Observed

### Init Sequence (on window reload)

1. `getProfile` - own user profile
2. `getEncryptedIdentityV3` - E2EE identity (wrappedNonce, kdfParameters)
3. `getServerTime` - server timestamp
4. `getLastOpRevision` - last operation revision number (for long polling)
5. `getConfigurations` - feature flags (huge map)
6. `getSettings` - user privacy/notification settings
7. `getE2EEPublicKeysEx` - E2EE public keys

### Contact/Chat Loading

8. `getAllContactIds` - list of friend MIDs
9. `getAllChatMids` - list of group chat MIDs
10. `getMessageBoxes` - conversation list with last messages
11. `getContactsV2` - contact details (batched)
12. `getMessageBoxesByIds` - specific conversation metadata
13. `getRecentMessagesV2` - recent messages for a conversation
14. `getChats` - group chat details (members, invitees)
15. `getMessageReadRange` - read receipts

### Send Message Flow

16. `negotiateE2EEPublicKey` - get recipient's public key
17. `determineMediaMessageFlow` - check media upload path
18. `sendMessage` - send encrypted message

### Real-time

- `fetchOps` - long poll (HTTP, 140s timeout) for new operations
- Not captured in HAR (was pending at export time)

## Request Authentication

Every API request to `line-chrome-gw` includes:

| Header | Value | Notes |
|--------|-------|-------|
| `cookie` | `lct=<JWT>` | Access token |
| `x-line-access` | `<JWT>` | Same token (some endpoints) |
| `x-hmac` | Base64 HMAC | Per-request signature |
| `x-lal` | `en_US` | Locale |
| `x-line-chrome-version` | `3.7.2` | Extension version |

### JWT Token Structure

```json
{
  "jti": "<uuid>",
  "aud": "LINE",
  "iat": <unix seconds>,
  "exp": <unix seconds>,          // ~7 day access token
  "scp": "LINE_CORE",
  "rtid": "<uuid>",
  "rexp": <unix seconds>,         // ~1 year refresh token
  "ver": "3.1",
  "aid": "<account MID>",
  "ctype": "CHROMEOS",
  "cmode": "SECONDARY",
  "cid": "<client id>"
}
```

## HMAC Computation (Critical)

Each request is signed with `X-Hmac`. The HMAC is computed inside `ltsm.wasm`, LINE's proprietary **white-box cryptography** module.

### Flow

```
Input: { path, body, accessToken }

1. derivedKey = SecureKey.deriveKey(SHA256("3.7.2"), SHA256(accessToken))
2. hmac = new Hmac(derivedKey)
3. message = TextEncoder.encode(path + (body ?? ""))
4. result = base64(hmac.digest(message))
```

Where `path` is e.g. `/api/talk/thrift/Talk/TalkService/getProfile` and `body` is the JSON request body string.

### SecureKey Initialization

```javascript
// Hardcoded per extension ID
const TOKEN = "wODdrvWqmdP4Zliay-iF3cz3KZcK0ekrial868apg06TXeCo7A1hIQO0ESElHg6D";
secureKey = wasmModule.SecureKey.loadToken(TOKEN);
versionHash = SHA256("3.7.2");  // version string
```

### Why This Can't Be Reimplemented

LTSM uses **white-box cryptography** ([TechVerse 2022 talk](https://speakerdeck.com/techverse_2022/javascript-whitebox-crypto-and-improving-security-for-line-chrome)):

- Keys are embedded in lookup tables (LUTs), never appear in plaintext
- Operations work on protected key objects, not raw bytes
- Even Curve25519 clamping uses 256-entry LUTs per byte
- Designed to resist Frida, DBI tools, and static analysis
- The 2.2MB WASM is mostly these lookup tables

**The WASM blob must be used as-is.** This is the core dependency.

## LTSM WASM Module Details

- **File:** `ltsm.wasm` (2,254,973 bytes)
- **Compiled with:** Emscripten (C++ via embind)
- **Functions:** 678
- **Imports:** 40 functions from Emscripten runtime (memory, syscalls, embind registration)
- **Exports:** 8 (memory, table, malloc, free, embind init, ctors, errno, type check)

### C++ Classes (from mangled symbols)

```
LTSM::SecureKey       - loadToken, deriveKey, exportKey
LTSM::Hmac            - digest
LTSM::AesKey          - AES operations
LTSM::Curve25519Key   - ECDH key exchange
LTSM::E2EEKey         - E2EE key wrapper
LTSM::E2EEKeychain    - Key chain management
LTSM::E2EEChannel     - encrypt/decrypt v1, v2
```

### Origin Check

The WASM calls back to JS to get `window.location.origin` and checks it against:

```
chrome-extension://ophjlpahpchlmihnnnihgmmeilfjmjjc  (main extension)
chrome-extension://jeafcjlknhfnfkfeliiolabnpjahldie  (alt/dev extension?)
```

This must be faked in the WASM host.

## E2EE (Letter Sealing)

Messages are end-to-end encrypted. The extension:

1. Calls `negotiateE2EEPublicKey(recipientMid)` to get the recipient's Curve25519 public key
2. Performs ECDH key exchange inside LTSM
3. Derives AES-CTR key via HKDF
4. Encrypts message, producing `chunks` (array of base64 strings)
5. Appends HMAC for integrity

Message format in `sendMessage`:
```json
{
  "contentType": 0,
  "contentMetadata": {"e2eeVersion": "2"},
  "chunks": ["<nonce>", "<ciphertext>", "<counter>", "<tag1>", "<tag2>"],
  "hasContent": false
}
```

E2EE encrypt/decrypt is also inside `ltsm.wasm` (`encryptV2`/`decryptV2`), so it requires the WASM blob too.

## Extension File Structure

```
3.7.2_0/
├── ltsmSandbox.html          # Sandbox iframe for WASM
├── popup.html                # Extension popup
├── static/
│   ├── js/
│   │   ├── main.js           # Main app (3.7MB, minified+obfuscated)
│   │   ├── ltsmSandbox.js    # WASM loader + embind glue (2.4MB)
│   │   ├── ltsm.wasm         # White-box crypto module (2.2MB)
│   │   ├── cropperSandbox.js # Image cropping
│   │   └── *.chunk.js        # Code-split chunks
│   ├── css/
│   └── media/
```

## Architecture for a Chromeless Client

```
┌──────────────────────────────────┐
│         Chromeless Client        │
│       (Rust, iced/slint GUI)      │
├──────────────┬───────────────────┤
│  LINE API    │  LTSM WASM Host   │
│  (JSON/HTTP) │  (wasmtime/wasmer)│
│              │                   │
│  - getProfile│  - loadToken()    │
│  - sendMsg   │  - deriveKey()    │
│  - fetchOps  │  - Hmac.digest()  │
│  - contacts  │  - E2EE encrypt   │
│  - etc.      │  - E2EE decrypt   │
└──────┬───────┴─────────┬─────────┘
       │                 │
       ▼                 ▼
  line-chrome-gw     ltsm.wasm
  (HTTPS/JSON)    (white-box crypto)
```

### Implementation Plan

**Phase 1: WASM Harness**
- Load `ltsm.wasm` in wasmtime or wasmer
- Stub ~40 Emscripten imports (most are embind type registration, memory ops)
- Fake origin check to return the extension's `chrome-extension://` URL
- Verify: call `loadToken` → `deriveKey` → `Hmac.digest` and compare output against a known request from the HAR

**Phase 2: API Client**
- Implement HTTP client talking JSON to `line-chrome-gw.line-apps.com`
- Extract JWT from browser (one-time), store locally
- Sign requests with WASM HMAC
- Implement init sequence, contact loading, message history

**Phase 3: Real-time**
- Implement `fetchOps` long polling loop
- Handle operation types (incoming messages, read receipts, friend updates)

**Phase 4: E2EE**
- Use LTSM WASM for `encryptV2`/`decryptV2`
- Implement the chunk format for send/receive
- Handle `negotiateE2EEPublicKey` and `getEncryptedIdentityV3`

**Phase 5: GUI**
- Lightweight native GUI (iced or slint)
- Contact/group list sidebar, chat view, message input
- `.desktop` file, system tray icon, D-Bus notifications, XDG autostart

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| LINE updates WASM blob | HMAC breaks | Swap in new `ltsm.wasm` from updated extension. API is embind-based, likely stable. |
| LINE changes gateway API | Client breaks | Monitor extension updates. JSON API paths are descriptive, easy to diff. |
| LINE detects non-extension client | Account ban | Match Chrome UA, version header. Skip telemetry or replicate it. |
| JWT expires | Auth fails | Implement token refresh flow (refresh token has ~1 year expiry). |
| Extension discontinued | No more blobs | Unlikely short-term. LINE has no native Linux client, Chrome ext is their Linux story. |
| Emscripten version changes | WASM host breaks | Import stubs may need updating. Embind ABI is fairly stable. |

## Comparison with Old Protocol Knowledge

| Aspect | Old docs (2018) | Current Chrome (2026) |
|--------|-----------------|----------------------|
| Wire format | Binary Thrift Compact | JSON |
| Gateway | `gd2.line.naver.jp` | `line-chrome-gw.line-apps.com` |
| Paths | `/S4`, `/P4` | `/api/talk/thrift/Talk/TalkService/{method}` |
| Auth | Raw token in header | JWT (HS256) |
| Request signing | None documented | HMAC via white-box WASM |
| E2EE | v1 (secp256k1) | v2 (Curve25519, white-box) |
| Real-time | Long poll (`fetchOps`) | Long poll (`fetchOps`) - unchanged |
| Client type | Primary | Secondary device |

## Motivation

The Chrome extension has real usability problems on Linux:

- **No autostart** - doesn't relaunch when Chrome restarts, must be manually reopened each time
- **No taskbar identity** - shows as generic "Wayland app" in KDE, no icon, no `.desktop` entry
- **No system notifications** - or at best, Chrome's notification system which is unreliable on Wayland/KDE
- **Heavy runtime** - requires an entire Chrome instance running for a text chat client
- **No window management integration** - can't pin to taskbar, no system tray icon

A native client fixes all of these with a proper `.desktop` file, XDG autostart, D-Bus notifications, and a system tray icon.

## Windows Desktop Client (Not Useful)

The Windows LINE client (v9.13, found at `AppData/Local/LINE/`) was examined and is **not relevant** to this project:

- **Qt6 native app** (30MB `LINE.exe`) with LTSM compiled as native x86-64 code
- Uses **OpenSSL 1.1** and likely the **binary Thrift protocol** (old `/S4`, `/P4` paths)
- Custom encrypted databases (`.edb` files)
- Running under WINE would bring massive dependencies for features we don't need (calls, timeline, shop, stickers rendering, WebView2)
- LTSM is baked into the PE binary - can't extract and reuse on Linux

The Chrome extension's WASM blob remains the only viable LTSM artifact for cross-platform reuse.

## Recommended Tech Stack

**Language:** Rust

**Dependencies:**
- `wasmtime` - WASM runtime for hosting `ltsm.wasm` (Emscripten embind stubs)
- `reqwest` - HTTP client for LINE JSON API
- `iced` or `slint` - GUI framework (lightweight, native, no webview)
- `tokio` - async runtime (long polling, concurrent API calls)
- `serde` / `serde_json` - JSON serialization
- `keyring` or file-based - JWT token storage
- `notify-rust` - D-Bus desktop notifications
- `dirs` - XDG directory paths

**Target feature set (MVP):**
1. Contact and group list
2. Chat view (text messages, timestamps, read receipts)
3. Send text messages
4. Real-time receive via `fetchOps` long polling
5. System tray icon
6. Desktop notifications for new messages
7. XDG autostart

**Non-goals for MVP:**
- Sticker rendering (show name/alt text instead)
- Image/file sending (receiving can show thumbnails)
- Voice/video calls
- Timeline/stories
- Shop

**Desktop integration:**
- `.desktop` file with proper icon and `StartupWMClass`
- XDG autostart entry (`~/.config/autostart/`)
- System tray via `ksni` or `tray-icon` crate
- D-Bus notifications via `notify-rust`

## Conclusion

The chromeless client approach is viable. The protocol is clean JSON, well-structured, and the WASM blob provides all needed crypto without requiring reimplementation. The main work is:

1. Building an Emscripten-compatible WASM host (~40 import stubs)
2. A straightforward HTTP client for the JSON API
3. Long polling for real-time updates
4. A lightweight GUI (iced/slint) with proper Linux desktop integration

The blob-swapping strategy is sound: as long as LINE keeps the same embind API surface (`SecureKey.loadToken`, `deriveKey`, `Hmac`, `E2EEChannel.encryptV2/decryptV2`), updating the WASM when the extension updates should be a drop-in replacement.
