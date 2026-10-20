# 閃 desktop handoff

## Architecture

- `sen` (root crate): `src/runtime.rs` hosts the original WASM;
  `src/crypto_module.rs` discovers and validates the installed module;
  `src/client/` implements pairing, signing, RPC, SSE and media;
  `src/backend/` provides LineBackend and the feature-gated FakeBackend.
- `src/engine.rs` schedules work on a store thread that alone owns SQLite.
  A separate `sen-net` thread owns the backend, WASM runtime and RPC work.
  SSE, asset fetchers and uploads have separate workers. Results return to
  the engine before FIFO deltas reach `src/controller.rs` on the UI thread.
- `sen-desktop` (`app/`) owns startup, callbacks and presentation.
  `app/src/presenter.rs` updates keyed Slint models from controller state.
  `sen-ui` (`ui/`) compiles the Slint sources in `app/ui/`.
  Winit uses the software renderer.

Transport is hyper HTTP/1 with async-io and verified rustls/WebPKI TLS.
One process-wide executor drives connections; blocking worker callers await
request futures. RPC, SSE and media use separate clients/pools. DNS uses
async-net's system lookup. Cancellation and deadlines cover requests and bodies.

Startup selects the newest installed LINE Chrome extension across Chrome and
Chromium profiles under `$XDG_CONFIG_HOME` (default `$HOME/.config`). Its
`static/js/ltsm.wasm` supplies crypto and its manifest version supplies client
versioning. Required interfaces are checked before sign-in. `--wasm` and
`--client-version` are explicit developer overrides; no downloaded copy is
selected by default.

Account history, drafts, preferences and outbox use an unencrypted, MID-scoped
SQLite database at `$XDG_DATA_HOME/sen/<mid>/store.sqlite3`. The last account
is plain MID text at `sen/auth/last-account`. Session credentials
are sealed with a desktop Secret Service key; media caches are private files.
The `demo` feature enables synthetic backends and snapshots with in-memory SQLite.
`ui-debug` includes `demo` and loopback MCP; normal builds omit both.

## Build and checks

```sh
cargo build --offline --release -p sen-desktop
cargo fmt --all -- --check
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
cargo build --offline --release -p sen-desktop --features demo
target/release/sen --demo --snapshot /tmp/line-ui.ppm
cargo build --offline --release -p sen-desktop --features ui-debug
python3 tools/check-ui-mcp.py --all
```

The checker builds nothing and owns private headless demo instances. Focused
checks use `--group NAME`; `--wide` and `--screenshots DIRECTORY` are optional.
Install a featureless release build with `python3 tools/install-desktop.py`.
The installer rejects demo and MCP builds.

Open issues are tracked in [docs/issues.md](docs/issues.md). See
[docs/core-model.md](docs/core-model.md) for ownership details,
[docs/ux-model.md](docs/ux-model.md) for behavior and
[docs/ux-acceptance.md](docs/ux-acceptance.md) for verification.

## Security constraints

**Chrome owns the secondary-device slot and native authorization is invalid.**
Do not run the default app, `--check-live`, pairing, sends,
read markers, or server preference changes without a new device-slot assignment.
Preserve `artifacts/auth/session.json` and all encrypted local state. App
defaults use `$XDG_DATA_HOME/sen/auth/session.json`
(fallback `$HOME/.local/share/sen/auth/session.json`). Crypto comes from the newest installed LINE extension under `$XDG_CONFIG_HOME` (default `$HOME/.config`), at its version directory’s `static/js/ltsm.wasm`. Historical
live results below do not prove current authorization.


Agent-started GUI previews must use `tools/run_ui_debug.py` with a `ui-debug`
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


The explicitly selected repo session `artifacts/auth/session.json` contains
**real access/refresh credentials and wrapped
keys**, in plaintext form (usable only with the plaintext-file development store). It is mode 0600, and its newly created parent directory is mode 0700.
Sealed sessions are only readable with their key in the desktop keyring: do not
delete `sen` keyring items.
Preserve it. Do not dump it, put it into prompts/logs, commit it, or copy its
contents into documentation. HAR files under `dumps/` are sensitive too.


Safe inspection: parse JSON and output only booleans, key counts, field names,
and file modes.

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

The repo has no remote. State/artifacts/captures/caches/build outputs/local
agent configs are ignored. Preserve the official blob and state on disk.

Do not introduce a JS runtime into the app. Temporary Node diagnostics used during
development were only a comparison against original Chrome bindings.

Do not add legacy `sandbox_mode` or `[sandbox_workspace_write]` settings: loaded
legacy settings take precedence over `default_permissions`. Do not launch with
`--sandbox` when intending to use this profile.

Do not reintroduce transport
spooling/proxy workarounds.

If another pairing is required, use the app's Sign in flow. It preserves the
previous session before installing the newly validated one. Pairing uses the
Chrome secondary-device slot and requires an explicit device-slot assignment.

Colour emoji: the app downloads the current bitmap Noto Color Emoji and OFL from
jsDelivr when absent, checks conditionally at most every 30 days, validates without
pinned hashes, and refreshes text live. Fonts are never embedded.
`python3 tools/fetch_emoji_font.py` (python3-fonttools) fetches the same source to
`fonts/`; the UI launcher/checks set `SEN_EMOJI_FONT`, disabling all font network work.
`tools/install-desktop.py` can install both files. The colour render regression
reads `fonts/NotoColorEmoji.ttf` directly and requires this local fixture.
Rebase both `third_party/` patches on every Slint upgrade.
