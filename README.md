# Native LINE client

A Rust client experiment using LINE Chrome's unmodified crypto WASM, with a
Slint desktop sketch. Linux desktop is the first target.

The protocol PoC has authenticated a real phone-paired session, restored three
Letter Sealing keys, and decrypted a real V2 chat payload containing a media key.
The desktop app currently uses example conversations only. It does not read the
saved session, connect to LINE, or send messages.

## Run the desktop sketch

```sh
cargo run --release -p line-desktop
```

Search and select conversations, type into the multiline composer, and switch
between chats to keep separate drafts in memory. Send is disabled. Drafts are
discarded when the window closes. The Japanese example text is synthetic.

The app explicitly selects Winit with Slint's software renderer. Default Slint
features are disabled; GPU renderers and Qt are not enabled. Both Wayland and
X11 are supported by the chosen backend. Accessibility is enabled. The desktop
compositor may use a GPU; the application renders its own pixels on the CPU.

For a headless rendering of the same window:

```sh
cargo run --release -p line-desktop -- --snapshot /tmp/line-preview.ppm
```

This renders through Slint's software renderer without creating a desktop window.
It checks layout and text rendering, not desktop input methods or window behavior.

## Protocol tools

The root package remains the default Cargo target, so ordinary CLI builds do not
compile the GUI. Obtain the official extension WASM separately; see
[HANDOFF.md](HANDOFF.md) for its supported version, hash, state handling, and
verified protocol details. The blob is not included in Git.

```sh
cargo build --release --bins
target/release/line-auth status
target/release/line-auth chat-check
cargo run --release --bin line-wasm-poc
```

`login` performs QR pairing when needed; preserve an existing session. `status`
checks saved authentication and key restoration. `chat-check` performs a bounded
read-only decryption probe and reports counts without saving message contents.

Credentials and wrapped keys under `artifacts/auth/`, proprietary extension
artifacts, and sensitive HAR captures under `dumps/` are ignored by Git. The
checked-in crypto fixture is synthetic. This is a local repository; no remote is
configured.

## Development

```sh
cargo fmt --all --check
cargo test --release --workspace
cargo clippy --release --workspace --all-targets -- -D warnings
```

The app sketch and planned integration are described in
[docs/app-design.md](docs/app-design.md). IME acceptance must be verified on the
actual desktop before treating the GUI as ready for messaging.
