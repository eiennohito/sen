# Build iteration

How to keep edit-build-check cycles short: the `fast` profile, the UI crate
split, the UI checker and what makes generated Slint code expensive.

## Profiles and crates

Use `cargo build --profile fast --offline -p sen-desktop` for local work.
`fast` inherits release, enables incremental compilation, and sets opt-level 0
for `sen-desktop` and `sen-ui`. Dependencies, including the software
renderer, protocol adapter and crypto, stay optimised. Use `release` for
release checks, crypto benchmarks and anything timing-sensitive.

The generated Slint modules belong to `ui/` (`sen-ui`); the app re-exports its
public API, so Rust-only app edits reuse the compiled UI. Slint edits still
rebuild it. Sources stay in `app/ui/` to preserve imports and icon paths.
`ui/build.rs` tracks that directory and `app/assets/`, including additions;
slint-build also tracks imported files and embedded resources. Keep the small
Slint test in `app/src/tests/rich_flow.rs` with its Rust tests.

Both builds compile `app/ui/app.slint`. The `@demo` library resolves to
`app/ui/product` (empty) or, with the `demo` feature, `app/ui/dev`; no source
is generated or rewritten before compilation. Editors running the Slint LSP
should map the `demo` library to `app/ui/dev`.

## UI checks

Build once and pass that binary explicitly:

```sh
cargo build --profile fast --offline -p sen-desktop --features ui-debug
python3 tools/check-ui-mcp.py --group rich --binary target/fast/sen
python3 tools/check-ui-mcp.py --all --binary target/fast/sen
```

The checker builds nothing. It owns headless demos with temporary XDG data
and cache directories and a private D-Bus session, waits up to 30 seconds for
MCP `initialize`, and resets synthetic state between groups in each window.
`--all` runs the normal (980 px), wide (1280×900) and narrow matrices plus
separate signed-out, missing-module and virtual-wheel launches, in a random
order it prints as a seed (`--seed N` repeats it). `--wide` selects 1280×900
for individual groups; `--screenshots DIRECTORY` captures surfaces. Release is
the default binary and is required for final timings. `ui-debug` forwards to
the UI crate's `slint/mcp`; `--build-info` and the installer's rejection of
debug builds still apply.

## Generated code size

Generated Rust, not Slint source lines, drives UI build time. Layout,
geometry and initialisation dominate it; bytes do not predict LLVM work
exactly, but large expansions are visible in both. What expands most:

- reusable components that Slint inlines (root `opacity`, `visible`, shadow,
  animated `background`, percentage sizes, instance `clip`, filled
  `@children`): every inlined copy duplicates their `if`s, popups and
  tooltips (see the last row of the patterns table in
  [deslop.md](deslop.md));
- `ScrollView`s, each of which expands native scrollbars;
- deep layout nesting with many children, and conditional wrapped content;
- key handlers with many early returns, which become nested tuples and
  clones.

Native Button, CheckBox and ComboBox already share implementations; extra
wrappers add forwarding and layout risk. Child-accepting ScrollView/Pane
components inline in Slint 1.18.1. A neutral root helps repeated components
such as TextMetrics and IconButton. After a size reduction, validate with
exact headless pixels and the full MCP matrix, and time warm Slint and Rust
edits. Measure runtime too after crate-boundary changes: cross-crate calls can
lose inlining without LTO.
