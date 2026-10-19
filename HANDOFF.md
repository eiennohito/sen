# LINE native client prototype: fresh-session handoff

Updated 2026-10-18. Workspace: `/home/eien/work/misc/line-app`.

The historical `line-auth` CLI described below has been removed; use the desktop app for sign-in.

## Decisions taken 2026-10-18

1. **T11 is implemented with the desktop Secret Service** (see the T11 row in
   [docs/ux-sessions.md](docs/ux-sessions.md) and `src/credentials.rs`). The
   session file is sealed and its key is kept in the keyring.
   `LINE_DESKTOP_CREDENTIALS=plaintext-file` is the development store for
   agent/debug instances (one plaintext file per `--state` path, as
   requested). The repo's `artifacts/auth/session.json` is still plaintext and
   untouched; keyring mode refuses plaintext files. Only the explicit
   plaintext-file development store can open it.
2. **T5, P1–P3, P5–P7 traces came from a live Chrome capture driven by the
   user (done 2026-10-18).** Results: [docs/protocol-traces.md](docs/protocol-traces.md);
   tool: `node tools/capture-line-protocol.mjs` (shapes only; control by
   appending `mark`/`shot`/`stop` lines to `<run>/control`). Raw shape traces
   and U0 reference screenshots are local in `artifacts/captures/`. Still
   untraced: others' reactions (op 140), invite/remove/accept/reject, block.

3. **Local data uses a durable, unencrypted SQLite store; only the session
   stays sealed.** Core model phases 1–3 implement the store, engine,
   controller and presenter, resolve D0–D8 and M13/M14, and use FakeBackend for
   demos and snapshots. See [docs/core-model.md](docs/core-model.md).
   C6/C7 and U0–U6 remain open; protocol work is tracked separately.

Next work is listed in [docs/ux-sessions.md](docs/ux-sessions.md).

## Session workflow

A Claude Code session is the coordinator. It hands implementation to Codex
sessions and then has independent reviewers check the result.

- **Worktrees.** Each implementation task gets its own worktree under
  `/home/eien/work/misc/line-app-wt/<name>`. The coordinator creates and removes
  them. The Claude project settings list that directory as an additional working
  directory, so reviewers can read it without permission prompts.
- **Git belongs to the coordinator.** Codex and reviewers do not run git commands
  that change state. The coordinator commits in the worktree, merges into `main`,
  and cleans up. To discard an attempt, use `git worktree remove --force` and
  `git branch -D`, not `git reset --hard` or `git clean`, because those ask for
  confirmation.
- **Codex launch.** Codex permissions live in profile files, not on the command
  line. The project profile is `~/.codex/line-app.config.toml`, which applies
  `workspace-net` (workspace writes plus network). Launch Codex with only a
  profile and a working directory:

  ```sh
  codex exec -p line-app -C /home/eien/work/misc/line-app-wt/<name> "<task>"
  ```

  Do not pass `-s`/`--sandbox`, `-c`, `--add-dir`, or approval flags. If a task
  needs different access, write another `~/.codex/<name>.config.toml` and
  select it with `-p`. Profiles are needed because worktrees lack the
  git-ignored `.codex/config.toml`, and only the main checkout is trusted.
- **No prompts mid-run.** Use absolute paths (`git -C <wt>`,
  `cargo … --manifest-path <wt>/Cargo.toml`, `codex exec -C <wt>`) instead of
  `cd <wt> && …` chains. The local settings allow `cargo build/test/clippy/fmt`;
  `cargo run` and `target/release/*` are deliberately not pre-allowed. Their
  live-account restrictions are described below.
- **Review.** After Codex finishes, independent reviewer subagents check the
  worktree diff before the coordinator commits and merges.

## Current state and authorization — read this first

The implementation and verification record is [docs/ux-acceptance.md](docs/ux-acceptance.md),
with the audited list of open problems in [docs/ux-sessions.md](docs/ux-sessions.md). The current
implementation includes core model phase 3: SQLite owns messages and account
state; the engine merges every input, the controller applies FIFO deltas, and
the presenter updates keyed Slint models. Messages have no retention limit.
Read the acceptance rows for partial behavior and verification; this remains
a prototype, not a daily-use or live-compatibility claim.

[docs/ux-model.md](docs/ux-model.md) defines behavior;
[docs/app-design.md](docs/app-design.md) owns engineering constraints and
[docs/chromium-ui-map.md](docs/chromium-ui-map.md) records source evidence.

**Chrome owns the secondary-device slot and native authorization is invalid.**
Do not run the default app, `--check-live`, pairing, sends,
read markers, or server preference changes without a new device-slot assignment.
Preserve `artifacts/auth/session.json` and all encrypted local state. App
defaults use `$XDG_DATA_HOME/line-desktop/session.json`
(fallback `$HOME/.local/share/line-desktop/session.json`). Crypto comes from the newest installed LINE extension under `$XDG_CONFIG_HOME` (default `$HOME/.config`), at its version directory’s `static/js/ltsm.wasm`. Historical
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
# Run each group on a fresh instance (the helper invokes run-ui-debug.py):
python3 tools/check-ui-mcp.py --launch --port 9448 --scenarios
# Repeat separately for --reading --rich --layers --friends --focus
# --layout-photos --media --unread --auth-recovery, one flag per fresh launch.
```

Agent-started GUI previews must use `tools/run-ui-debug.py` with a `ui-debug`
build and `dbus-run-session`: Slint's headless software backend runs in private
0700 XDG directories and a D-Bus session with no activation service directories.
Use a unique port and binary copy per session; the wrapper reports owned
app/scope PIDs, removes its PID file and temporary directories on exit, and
supplies guarded `--debug-background`. Stop only that owned scope. Normal
launches keep winit and foreground startup.

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

The app uses Slint 1.18.1 Winit/software renderer. One engine worker owns the
SQLite store and LineBackend's crypto/RPC work. A cancellable SSE thread feeds
bounded live operations to the scheduler. The UI controller owns AppState and
drains FIFO deltas on a 16 ms timer; the presenter alone updates keyed Slint
models. FakeBackend runs demos and snapshots through this same path with an
in-memory SQLite store. Account stores are MID-scoped databases under XDG data;
validated sessions can open stored content before connecting. Asset fetchers
and desktop bus work run separately.
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
reaction frames (128 MiB / 4096 files disk LRU by mtime, refreshed on disk
hits with no age expiry; 24 MiB decoded memory). Standard
reactions still use Unicode substitutes; animations and custom sticker variants
are missing. Encrypted sends are wired but no automated real send was performed.

The repo has no remote. State/artifacts/captures/caches/build outputs/local
agent configs are ignored. Preserve the official blob and state on disk.

## Historical verified state (before Chrome login)

- Chrome extension version fetched for this prototype: **3.7.2**. Its unmodified WASM is
  `artifacts/line-3.7.2/ltsm.wasm`, 2,254,973 bytes, SHA-256
  `58bb4e189ab9bbd7d72ed415258da36afd50e306cb019bab75329d4f5f1b65b3`.
  The synthetic oracle fixture rejects other hashes; the app validates the installed module’s interfaces. This is the version obtained
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

## Historical authentication checks — native auth currently invalid

The removed CLI reimported saved wrapped keys into WASM, signed and sent
`getProfile`, and performed a bounded read-only decryption probe of recent
chat previews. The historical results above do not establish current account
authorization or validate token refresh.

The desktop now provides message retrieval, text display, and media decryption.
Successful samples do not cover every direction, chat type, historical key, or
content type. Current token refresh and live receive remain unverified against
the account.

## Sensitive local state

The explicitly selected repo session `artifacts/auth/session.json` contains
**real access/refresh credentials and wrapped
keys**, in plaintext form (usable only with the plaintext-file development store). It is mode 0600, and its newly created parent directory is mode 0700.
Sealed sessions are only readable with their key in the desktop keyring: do not
delete `line-desktop` keyring items.
Preserve it. Do not dump it, put it into prompts/logs, commit it, or copy its
contents into documentation. HAR files under `dumps/` are sensitive too.

Safe inspection: parse JSON and output only booleans, key counts, field names,
and file modes. The removed CLI wrote stage names and timestamps to
`artifacts/auth/session.progress.json`; the app reports progress in its sign-in UI.
`artifacts/` is ignored by `.gitignore`.

## Code and build

- `src/runtime.rs`: minimal WASM host, dynamic embind bindings, emval handling.
- `src/lib.rs`: shared version/hash constants, runtime module, request signing.
- `examples/crypto-harness.rs`: offline crypto/signature and benchmark harness.
- `src/client.rs`: shared protocol and persistence helpers.
- `src/client/pairing.rs`: app QR pairing flow.
- `src/client/chat.rs`: shared chunk/key adapters.
- `src/client/desktop.rs`: account operations used by the desktop worker.
- `src/client/desktop/assets.rs`: authenticated media retrieval, decryption, and
  private disk/decoded-image caches.
- `src/store/`, `src/engine.rs`, `src/backend.rs`: durable SQLite data, engine
  scheduler and LineBackend/FakeBackend.
- `src/controller.rs`, `app/src/presenter.rs`: AppState projection and keyed Slint models.
- `app/`: Slint GUI, original app icon, FakeBackend demo, headless snapshots.
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
cargo run --offline --release --features harness --example crypto-harness
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
   The fixed-offset WASM-memory helper in the offline harness exists only to build
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

Codex sessions launched in worktrees use the `~/.codex/line-app.config.toml`
profile instead (see "Session workflow"). The project file below still applies
to Codex sessions started in the main checkout.

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

If another pairing is required, use the app's Sign in flow. It preserves the
previous session before installing the newly validated one. Pairing uses the
Chrome secondary-device slot and requires an explicit device-slot assignment.
