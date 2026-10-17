# LINE native client PoC: fresh-session handoff

Updated 2026-10-17. Workspace: `/home/eien/work/misc/line-app`.

## Current state and authorization — read this first

The implementation and verification record is [docs/ux-acceptance.md](docs/ux-acceptance.md),
with the audited list of open problems in [docs/ux-sessions.md](docs/ux-sessions.md). The current
implementation is committed as `d1e5db5`, with browser keyboard/metadata, native
composer state and failure handling, reading and rich content, layered surfaces,
unread eligibility, attention services, and bounded older-history work. Read the
individual acceptance rows for partial behavior and tested builds; this remains
a PoC, not a daily-use or live-compatibility claim.

[docs/ux-model.md](docs/ux-model.md) defines behavior;
[docs/app-design.md](docs/app-design.md) owns engineering constraints and
[docs/chromium-ui-map.md](docs/chromium-ui-map.md) records source evidence.

**Chrome owns the secondary-device slot and native authorization is invalid.**
Do not run the default app, `line-auth status`, `--check-live`, pairing, sends,
read markers, or server preference changes without a new device-slot assignment.
Preserve `artifacts/auth/session.json` and all encrypted local state. CLI
defaults use `$XDG_DATA_HOME/line-desktop/session.json` and `ltsm.wasm`
(fallback `$HOME/.local/share/line-desktop/`) and follow the app's saved paths
choice. Historical
live results below do not prove current authorization.

Safe build and verification commands (synthetic data only):

```sh
cargo build --release --offline -p line-desktop
cargo fmt --all --check
cargo test --release --offline --workspace
cargo clippy --release --offline --workspace --all-targets -- -D warnings
target/release/line-desktop --demo --snapshot /tmp/line-ui.ppm
cargo build --release --offline -p line-desktop --features ui-debug
cp target/release/line-desktop /tmp/line-ui-debug-9447
python3 tools/run-ui-debug.py --binary /tmp/line-ui-debug-9447 --port 9447 --log /tmp/line-ui-9447.log -- --demo
# In another terminal:
python3 tools/check-ui-mcp.py --port 9447 --scenarios --layout-photos --friends --auth-recovery --reading --layers --rich
```

Agent-started GUI previews must use `tools/run-ui-debug.py`: it uses a private
D-Bus session and virtual KWin output, with no host desktop window or focus
request. It requires `kwin_wayland`, `dbus-run-session`, and a `ui-debug` build;
it refuses to fall back to the host display. The guarded `--debug-background`
flag is supplied by the wrapper, not ordinary launches. Use a unique port and
binary copy per session; the wrapper reports its owned app/scope PIDs and a
unique PID file. Stop only that owned scope. Normal user launches remain
foreground. Headless `--snapshot` does not map a GUI window.

Snapshots, offline demos, tests, and synthetic WASM benchmarks are safe.
Physical IME, compositor input, scaling, and notification checks remain the
person-only V session. Read each session's hand-off before continuing; record
new evidence in acceptance rows and the verification log.

Chrome debugging was previously approved on 127.0.0.1:9222; it is unnecessary
for offline UI work. If reference inspection is needed, use
`node tools/chrome-ui-map.mjs --interactive` and preserve privacy: no account
storage dumps, messages, preference changes, or private captures in Git.

## Current limits and next verification

Physical Fcitx composition, real scrollbar dragging, compositor activation,
100/150/200 % scale, desktop notification interactions, and connected resource
measurements remain release gates. Synthetic input and screenshots do not satisfy
those gates. Source-backed SSE, token refresh, read markers, receipts and older
paging exist, but account compatibility has not been exercised after Chrome took
the device slot. Unknown native mute state remains unknown.

Reading and composition preserve local text independently of delivery results.
Sent and Read are distinct; Rejected and Unconfirmed have different recovery
actions. Rich inline-image messages offer Select text for intact fallback-body
selection; cross-run drag selection and contextual shaping across grapheme runs
remain toolkit deviations. The acceptance record names the remaining limitations.

Next: finish the remaining session handoffs and collective offline checks, then
the person-run V checks. Restore native account authorization only after the user
explicitly assigns its device slot; no historical authorization carries forward.

## Existing app and tooling

The app uses Slint 1.18.1 Winit/software renderer. The account worker owns crypto
and RPC/media work; a separate cancellable SSE thread supplies operations through a
bounded queue. UI models stay on the UI thread; desktop bus work is separate.
Normal builds omit MCP;
`--features ui-debug` enables loopback inspection through the guarded private
`tools/run-ui-debug.py` scope above; use a unique port for each owner.
`tools/check-ui-mcp.py` checks synthetic browser, reading, media, layer and
rich-content flows; focused browser/composer scripts cover their keyboard paths.
Actual Fcitx preedit/candidate acceptance is still pending.

The previously installed user executable/icon/launcher are from the earlier
integration work; source changes do not update that installation automatically.
Build with `cargo build --release -p line-desktop`, then
`python3 tools/install-desktop.py` when an installation is wanted. The installer
rejects MCP debug builds. Do not launch the native account during the audit.

Private static image caching already covers avatars, photos, stickers, and paid
reaction frames (128 MiB / 4096 files disk, 24 MiB decoded memory). Standard
reactions still use Unicode substitutes; animations and custom sticker variants
are missing. Encrypted sends are wired but no automated real send was performed.

The repo has no remote. State/artifacts/captures/caches/build outputs/local
agent configs are ignored. Preserve the official blob and state on disk.

## Historical verified state (before Chrome login)

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

## Historical resume commands — native auth currently invalid

Do not run the native authenticated commands below during the Chrome audit.
The following records the previous workflow. An HTTP response of any status from this connectivity
probe establishes DNS/TLS/HTTPS connectivity; the gateway root need not return 200.

```sh
printenv CODEX_SANDBOX_NETWORK_DISABLED
curl --silent --show-error --connect-timeout 10 --max-time 15 \
  --output /tmp/line-gateway-connectivity.txt \
  --write-out 'LINE gateway HTTP %{http_code}\n' \
  https://line-chrome-gw.line-apps.com/
target/release/line-auth status --state artifacts/auth/session.json --wasm artifacts/line-3.7.2/ltsm.wasm
```

`status` reimports the saved wrapped keys into WASM, signs and sends `getProfile`
with the saved token, checks that a profile MID exists, and saves `validated=true`
only after success. It does not print profile contents or tokens. On server
rejection, diagnose the actual response before deciding whether re-pairing is
necessary. The current source has a sourced token-refresh adapter; these
historical checks do not validate it.

`target/release/line-auth chat-check --state artifacts/auth/session.json --wasm
artifacts/line-3.7.2/ltsm.wasm` reads at most ten chat previews with five
messages each, tries at most five supported candidates, and stops on the first
verified JSON payload. It reads credentials without rewriting them and does not
send messages, mark them read, register keys, or persist chat data. Each candidate
uses a fresh runtime, so a WASM exception cannot poison the next attempt.

The desktop now extends this into real message retrieval, text display, and
media decryption. Successful samples do not cover every direction, chat type,
historical key, or content type. Current token refresh and live receive remain
unverified against the account.

## Sensitive local state

The explicitly selected repo session `artifacts/auth/session.json` contains
**real access/refresh credentials and wrapped
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

Current test totals and exact verified builds are recorded in the acceptance
verification log. Earlier real read-only checks are historical and do not
establish current authorization. The signature replay additionally
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
It supports `--display ascii|none`, `--qr-file`, and `--wasm`. Pairing uses
the Chrome secondary-device slot; this consequence was already discussed and
authorized by the user.
