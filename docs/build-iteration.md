# Build iteration

Use `cargo build --profile fast --offline -p line-desktop` for local work.
`fast` inherits release, enables incremental compilation, and sets opt-level 0
for `line-desktop` and `line-desktop-ui`. Dependencies, including the software
renderer, protocol adapter and crypto, remain optimised. Use `release` for the
release gates, crypto benchmarks and any timing-sensitive work.

The generated modules belong to `ui/` (`line-desktop-ui`); the app re-exports
its public API so Rust-only app edits reuse the compiled UI. Slint edits still
rebuild it. Sources stay in `app/ui/` to preserve imports and icon paths.
The UI build script tracks that directory and `app/assets/`, including additions;
slint-build also tracks imported files and embedded resources. Keep the small
Slint test in `app/src/rich_content.rs` with its Rust tests.

For MCP iteration, build once and pass that binary explicitly:

```sh
cargo build --profile fast --offline -p line-desktop --features ui-debug
python3 tools/check-ui-mcp.py --group rich --binary target/fast/line-desktop
python3 tools/check-ui-mcp.py --all --binary target/fast/line-desktop
```

The check command builds nothing. It owns headless demos and temporary XDG data
and cache directories, waits for MCP `initialize`, and resets synthetic state
between groups in each window. `--all` includes the normal and wide matrix plus
separate signed-out and missing-module groups. `--wide` selects 1280×900 for an
individual group; `--screenshots DIRECTORY` captures surfaces. Release is the
default binary and is required for final timings. See the README for the full
command. The app forwards ui-debug to the UI crate’s `slint/mcp`; `--build-info`
and the installer’s debug-build rejection still apply.

## Generated source costs

The normal release generated Rust is roughly 7 MB. Representative whole-file
accounting (decimal MB, including generated formatting):

| Kind | Approximate MB | Share |
| --- | ---: | ---: |
| Layout, geometry and layout caches | 1.80 | 26% |
| Initialisation, constant markers, trackers and lifecycle | 1.43 | 20% |
| Other property binding closures | 0.95 | 14% |
| Item-tree tables and vtable forwarding | 0.60 | 9% |
| Callbacks and source functions | 0.50 | 7% |
| Accessibility methods | 0.48 | 7% |
| Repeater/subtree glue | 0.38 | 6% |
| Public Rust API | 0.23 | 3% |
| Focus/key handling | 0.22 | 3% |
| Fields, attributes, syntax and other support | 0.38 | 5% |

Classify generated functions and individual init statements by purpose;
attributes, fields and gaps stay in support. Counts include formatting (about
one third of bytes); quoted literals are under 1%. Bytes do not predict LLVM
work. A span from one generated struct to the next can include unrelated code:
the migration-error Text at `app/ui/app.slint:233` is about 21 KB, while its
apparent larger span includes AppWindow's flattened tables and public API.
Shared components still contribute physical items to root tables. The F12
`dialog` comes from `demo-scenarios.slint`; the shared ComboBox `popup` comes
from pinned Fluent `combobox.slint`, including its ScrollView/list model.

## Contributors and follow-up options

Rank contributors by source constructs rather than generated element suffixes:

1. `app/ui/layers.slint`: auth/profile/settings/viewer/cancel layouts contribute
   about 367 KB of layout/geometry. ScrollViews at lines 75, 120 and 163 expand
   six native scrollbars: about 179 KB of targeted init/bindings/handlers,
   excluding root tables and most geometry. Thumb bindings and pointer handlers
   cost about 2.3 KB and 3.1 KB each.
2. `app/ui/app.slint`: Friends/Conversation and child-accepting Pane layouts
   contribute about 235 KB of layout/geometry. Root item tables add roughly
   225 KB elsewhere. Extraction can redistribute costs without removing items.
3. `app/ui/message.slint:28,60,75,92`: nested outer/content/column/bubble layouts
   contribute about 114 KB, including 25 KB of constraint helpers and 20 KB of
   bubble cache/info bindings. Width bounds and conditional wrapped content
   drive expansion.
4. `app/ui/layers.slint:184–201,242–289`: notification settings alone generates
   28 KB in three layout bindings; the viewer adds a 29 KB constraint helper.
   Short layout declarations with many children have high byte costs per line.
5. Key handlers in `layers.slint:103–113,141–148,228–240`,
   `conversation.slint:75–87` and `friends.slint:52–60` generate 8–16 KB each.
   Early returns become nested tuples/clones. State's browser/reading handlers
   (`state.slint:24–44,231–249`) add 39 KB; callers invoke them rather than
   copying their bodies. State's public accessors/callback methods add 132 KB.

Native Button, CheckBox and ComboBox already share implementations; additional
wrappers add forwarding and layout risk. Child-accepting ScrollView/Pane must
inline in Slint 1.18.1, including facades. Neutral roots help repeated components
such as TextMetrics/IconButton; preserve inner visibility, input and sizing.

These follow-ups are unimplemented estimates, overlap, and must not be summed:

| Priority | Proposal | Estimated bytes saved | Constraint |
| --- | --- | ---: | --- |
| 1 | Share the scrollbar implementation behind a neutral root inside a local ScrollView implementation | 120–220 KB | A facade cannot share the existing body; maintain pinned styling, visibility, clipping, keyboard and drag rules. Per-instance state and root tables remain. |
| 2 | Remove redundant layout levels in settings/viewer, preserving explicit constraints | 40–90 KB | Preserve wrapped text measurement and stretch allocation; avoid guessed fixed heights. |
| 3 | Reduce message layout constraint passes | 25–50 KB | Keep measured widths and bubble height identical across plain/rich/reply fallbacks. |
| 4 | Model repeated settings CheckBox rows | 20–45 KB | Savings are parent layout/init/table entries; preserve row order, tab order and status text. |
| 5 | Separate key selection from action execution with one final EventResult | 15–35 KB | Reduce return-check scaffolding while keeping synchronous IME and focus behaviour. |

Validate with release gates, exact headless pixels and the full MCP matrix.
Time two warm semantic edits per profile for both Slint and app Rust, restoring
and warming between samples. Measure runtime costs after crate-boundary changes: cross-crate calls can lose
inlining without LTO.
