# LINE native client PoC: fresh-session handoff

Updated 2026-10-15. Workspace: `/home/eien/work/misc/line-app`.

## User intent and next action

**Latest request:** add an icon and connect the Slint UI to the real implementation,
including image/sticker/reaction caching. The user also pointed out Slint's MCP
feature; it is now available as opt-in `--features ui-debug`.

The default `line-desktop` now opens the saved account on a worker thread, loads
real conversation names and bounded recent history, decrypts through the original
WASM, and displays private cached assets. Refresh is manual; the current history
is capped at 50 messages. Conversation pages have a More button. Text sending is
wired to a separate Send action; **no automated real send has been performed**.
Enter remains owned by the multiline editor/IME. A timeout does not auto-retry or
clear the draft. Runtime exceptions replace the WASM instance.

The original chat-bubble SVG and matching Wayland/X11 app identity are installed
for the current user. Normal executable: `~/.local/bin/line-desktop`; launcher:
`~/.local/share/applications/org.lineclient.Desktop.desktop`. The launcher passes
absolute state/WASM paths. Installer: `python3 tools/install-desktop.py`; it rejects
UI-debug builds. The installed binary reports `mcp_debug=false` via `--build-info`.

Run from this repo: `cargo run --release -p line-desktop`. Use `--demo` for the
synthetic offline UI; `--snapshot FILE.ppm` always uses synthetic data. Override
paths with `--state FILE`, `--wasm FILE`, and `--cache-dir DIR`.

Live read-only verification: `target/release/line-desktop --check-live --cache-dir
artifacts/cache` loaded 8 chats and 68 messages across 3 chats. All 68 were readable;
62 encrypted payloads decrypted. Twelve assets decoded: 2 avatars, 5 photos,
3 stickers, and 2 paid-reaction images. Seven reaction instances were present.
A second run had 24 cache hits, zero misses/downloads. No contents/IDs/keys were
logged or saved outside the private thumbnail cache; no messages were sent.

The cache defaults to `$XDG_CACHE_HOME/line-desktop` (or `~/.cache/line-desktop`),
with hashed account directories (0700), hashed PNG filenames (0600), a 128 MiB /
4096-file disk cap and 30-day expiry, and a 24 MiB decoded worker cache. Photos use
OBS encrypted access tokens and verified HKDF/AES-CTR/HMAC media decryption
(the original Chrome app does this outside the LTSM blob too). Independent Go
media fixtures verify interoperability and modified-MAC rejection. Stickers and
paid reactions use static frames. Standard reactions use emoji.

MCP development workflow:
`SLINT_MCP_PORT=9447 cargo run --release -p line-desktop --features ui-debug -- --demo`
then `python3 tools/check-ui-mcp.py --screenshot /tmp/line-ui.png`.
It binds loopback `/mcp` and includes Slint debug element IDs. The script refuses
real-account windows. It verified selection/filtering, Japanese/emoji draft
retention, multiline input, and disabled demo Send on Wayland at scale 1.5.
The connected window also received real chats/messages with valid text geometry,
without dumping text or screenshots. Real Fcitx preedit/candidate testing remains
pending; do not claim perfect IME support. Normal builds omit MCP entirely.

The repo is on `main` with no remote. Sensitive state, captures, extension
artifacts, build outputs, caches under artifacts/, and local agent configs are
ignored. Source extraction moved the CLI implementation to `src/client.rs`, its
crypto adapters to `src/client/chat.rs`, and added `src/client/desktop.rs` plus
`src/client/desktop/assets.rs`. The CLI binary is a thin wrapper. See `README.md`
and `docs/app-design.md`. Next priorities: manual IME/send validation, token
refresh, automatic receive/reconnect, and older-history paging. The old 38 MiB
RSS sample below was the offline shell, not the connected account.

Continue the Rust client using the original Chrome extension's crypto WASM.
Real authentication and a real encrypted chat payload have now succeeded. The eventual goal is a native LINE client. The user
prefers retaining the official crypto blob for compatibility with future changes,
with a small Rust host/translation layer rather than a JS runtime or reimplemented
crypto. A debug CLI using kitty graphics for QR display is already implemented.

**First verify networking, then run `target/release/line-auth status`.** A real
pairing already produced credentials and three successfully unwrapped Letter
Sealing keys. Another QR scan should not be needed unless the saved credentials
have become invalid. Do not restart implementation or repeat pairing by default.

## Current verified state

- Chrome extension version fetched for this PoC: **3.7.2**. Its unmodified WASM is
  `artifacts/line-3.7.2/ltsm.wasm`, 2,254,973 bytes, SHA-256
  `58bb4e189ab9bbd7d72ed415258da36afd50e306cb019bab75329d4f5f1b65b3`.
  The runtime intentionally rejects other hashes. This is the version obtained
  during this work, not a claim that it will remain the newest available.
- Rust + Wasmi instantiate it without JavaScript. The host implements the needed
  Emscripten/embind/emval imports and callbacks.
- All **28/28 captured Chrome request signatures** matched the HAR.
- Original WASM decrypted independently generated Go Letter Sealing V1/V2
  fixtures. AES Unicode/NUL roundtrip, X25519, 64-bit sequence numbers, and
  rejection of modified tags/wrong sender IDs also passed.
- Real phone pairing reached session credentials. Original WASM decrypted the
  phone's encrypted keychain, recovered **three Letter Sealing keys**, and verified
  their export/import public-key matches. Saved keys also passed restoration.
- **Fresh-session verification on 2026-10-15:** normal sandbox HTTPS returned
  HTTP 200 from both example.com and the LINE gateway. `line-auth status`
  authenticated the saved session and restored all three keys.
- **Real chat decryption verified:** `line-auth chat-check` fetched eight chat
  previews (`hasNext=false`) and decrypted the first supported encrypted message
  with the original WASM. The V2 plaintext was a 62-byte JSON object containing
  `keyMaterial` (media/file key), with no text or location. No message contents,
  identifiers, or key material were logged or saved. This verifies decryption of
  an actual message payload. The connected desktop verification subsequently
  exercised real text, authenticated photo previews, stickers, and paid reactions
  as detailed above.

## Resume commands

Run from the workspace. An HTTP response of any status from this connectivity
probe establishes DNS/TLS/HTTPS connectivity; the gateway root need not return 200.

```sh
printenv CODEX_SANDBOX_NETWORK_DISABLED
curl --silent --show-error --connect-timeout 10 --max-time 15 \
  --output /tmp/line-gateway-connectivity.txt \
  --write-out 'LINE gateway HTTP %{http_code}\n' \
  https://line-chrome-gw.line-apps.com/
target/release/line-auth status
```

`status` reimports the saved wrapped keys into WASM, signs and sends `getProfile`
with the saved token, checks that a profile MID exists, and saves `validated=true`
only after success. It does not print profile contents or tokens. On server
rejection, diagnose the actual response before deciding whether re-pairing is
necessary; token refresh is not implemented.

`target/release/line-auth chat-check` reads at most ten chat previews with five
messages each, tries at most five supported candidates, and stops on the first
verified JSON payload. It reads credentials without rewriting them and does not
send messages, mark them read, register keys, or persist chat data. Each candidate
uses a fresh runtime, so a WASM exception cannot poison the next attempt.

The desktop now extends this into real message retrieval, text display, and
media decryption. Successful samples do not cover every direction, chat type,
historical key, or content type. Token refresh is still unimplemented.

## Sensitive local state

`artifacts/auth/session.json` contains **real access/refresh credentials and wrapped
keys**. It is mode 0600, and its newly created parent directory is mode 0700.
Preserve it. Do not dump it, put it into prompts/logs, commit it, or copy its
contents into documentation. HAR files under `dumps/` are sensitive too.

Safe inspection: parse JSON and output only booleans, key counts, field names,
and file modes. `artifacts/auth/session.progress.json` contains stage names and
timestamps only; consult it for the latest run's stage.
`artifacts/` is ignored by `.gitignore`.

## Code and build

- `src/runtime.rs`: minimal WASM host, dynamic embind bindings, emval handling.
- `src/lib.rs`: shared version/hash constants, runtime module, request signing.
- `src/main.rs`: offline crypto/signature PoC.
- `src/bin/line-auth.rs`: thin CLI wrapper. `src/client.rs` contains the shared
  transport, QR flow, state restoration, and persistence.
- `src/client/chat.rs`: shared chunk/key adapters and bounded decryption check.
- `src/client/desktop.rs`: account operations used by the desktop worker.
- `src/client/desktop/assets.rs`: authenticated media retrieval, decryption, and
  private disk/decoded-image caches.
- `app/`: Slint GUI, worker, original app icon, offline demo, headless snapshots.
- `tools/check-ui-mcp.py`, `tools/install-desktop.py`: UI checks and local install.
- `fixtures/letter-sealing-v2.json`: synthetic independent crypto fixture.
- `tools/fetch-extension.sh`, `tools/extract_extension.py`: extension acquisition.
- `tools/generate-fixture.go`: independent fixture generator.
- `licenses/beeper-line-MIT.txt`: attribution for binding conventions.
- `REPORT.md`: earlier viability analysis; prefer the verified status in this
  handoff where they differ. Do not treat its optimistic conclusions as tests.

Normal build/check commands when network access works:

```sh
cargo build --release --bins
cargo test --release --workspace
cargo clippy --release --workspace --all-targets -- -D warnings
cargo run --release --bin line-wasm-poc
```

Last checks include ten workspace tests (shared protocol/cache and stale UI response handling),
Clippy with warnings denied, MCP UI checks, real read-only checks, and the
offline crypto PoC. Formatting was applied. The signature replay additionally
accepts positional `WASM HAR` arguments; avoid printing HAR contents.

If dependency downloads are still unavailable, `/tmp/line-cargo-vendor` existed at
handoff time. Add these flags to Cargo commands before any `--` separator:

```sh
--offline \
--config 'source.crates-io.replace-with="local-vendor"' \
--config 'source.local-vendor.directory="/tmp/line-cargo-vendor"'
```

This is an optional build cache, not a repository dependency. No repository
configuration points to it.

## Important binding/protocol discoveries

1. Pairing metadata's `errorCode` is the string **`"SUCCESS"`**, not numeric zero.
   This was checked against the installed Chrome `main.js` and fixed in the CLI.
2. `E2EEKeychain.get()` returns an **emval handle containing a class pointer**,
   not a direct pointer. `Runtime::object()` extracts/releases that handle. This
   fix enabled real key recovery. Inherited methods also use registered upcasts.
3. Chrome V2 encryption/decryption takes API-provided **opaque encrypted MIDs**
   (44 characters), not raw 33-character LINE IDs. Pass real API IDs unchanged.
   The fixed-offset WASM-memory helper in the offline PoC exists only to build
   synthetic opaque IDs for tests; it is not a production identity mechanism.
4. A WASM exception poisons that runtime instance. Construct a new one before
   retrying; do not reuse it after a rejected crypto operation.
5. Reusing the blob does not guarantee arbitrary future protocol compatibility:
   request shapes, imports, exports, and binding names may require adapter updates.
6. Encrypted message chunks are base64: V1 WASM input concatenates chunks 0,1,2;
   V2 uses 0,2,1 (salt, nonce, ciphertext/tag). Chunks 3 and 4 are four-byte
   big-endian sender/receiver key IDs. V2 authenticates API `to`, `from`, both key
   IDs, and `contentType`. `getE2EEPublicKey` args are `[mid,1,keyId]`;
   `getE2EEGroupSharedKey` args are `[1,chatMid,groupKeyId]`.

Gateway: `https://line-chrome-gw.line-apps.com`. QR RPCs use JSON array bodies
`[argsObject]` and paths beneath
`/api/talk/thrift/LoginQrCode/SecondaryQrCodeLoginService/`; polling uses
`SecondaryQrCodeLoginPermitNoticeService/`. QR HMACs use an empty token argument.
Authenticated profile uses `/api/talk/thrift/Talk/TalkService/getProfile` with
body `[0]`, access-token HMAC input, `x-line-access`, and cookie `lct`.
The CLI contains the exact flow and headers.

Installed Chrome reference scripts, if needed:
`/home/eien/.config/google-chrome/Default/Extensions/ophjlpahpchlmihnnnihgmmeilfjmjjc/3.7.2_0/static/js/`.
Do not introduce a JS runtime into the app. Temporary Node diagnostics used during
development were only a comparison against original Chrome bindings.

## Sandbox configuration and verified profile (2026-10-15)

The project now uses permission profiles exclusively in `.codex/config.toml`:

```toml
default_permissions = "workspace-net"

[permissions.workspace-net]
description = "Workspace writes with direct network access."
extends = ":workspace"

[permissions.workspace-net.network]
enabled = true
```

Do not add legacy `sandbox_mode` or `[sandbox_workspace_write]` settings: loaded
legacy settings take precedence over `default_permissions`. Do not launch with
`--sandbox` when intending to use this profile. Official reference:
https://learn.chatgpt.com/docs/permissions

Installed CLI: 0.162.1. A fresh local app-server `config/read` resolved
`default_permissions = "workspace-net"`, both legacy fields null, and networking
enabled. `permissionProfile/list` included `workspace-net` as allowed.

An actual sandbox launched with
`codex sandbox -P workspace-net -C /home/eien/work/misc/line-app -- ...`
returned HTTP 200 from the LINE gateway. It allowed workspace and `/tmp` writes
and denied write opens on the project and user Codex configs. This preserves
filesystem sandboxing while enabling direct networking; no transport proxy is
configured. The legacy project configuration was backed up to
`/tmp/line-app-legacy-sandbox.config.toml`.

The previous conversation was started with the built-in `:workspace` profile,
confirmed by `CODEX_PERMISSION_PROFILE` and the actual sandbox process's
`network: restricted` policy. Editing the default has not changed that active
selection. Select `workspace-net` in the client permissions controls for the
conversation, or start a session that uses the configured default. Verify with
an actual curl request inside the normal tool sandbox. The network-disabled
environment flag can be inherited by a newly launched sandbox, so it alone is
not proof of the effective network policy.

The fresh conversation verified normal sandbox network access and successfully
ran both saved-session `status` and real-message `chat-check`, without transport
escalation. Cargo dependency caching needed approval because the user's Cargo
directory is outside the writable roots; this was granted. Dependencies are now
cached, and `--offline` builds/checks work. Do not reintroduce transport
spooling/proxy workarounds.

If another pairing is genuinely required, `line-auth login` refuses to overwrite
an existing state. Preserve the current state and use a different `--state` path.
It supports `--display kitty|ascii|none`, `--qr-file`, and `--wasm`. Pairing uses
the Chrome secondary-device slot; this consequence was already discussed and
authorized by the user.
