# UI latency audit (L2)

Measured offline in a release `ui-debug` build, using the software snapshot
backend at 980×720. The opt-in harness is in `app/src/latency.rs`:

```sh
cargo build --release --offline -p line-desktop --features ui-debug
LINE_UI_LATENCY=1 target/release/line-desktop --demo --snapshot /tmp/latency.ppm
```

The controlled fixture contains 300 chats, a 500-message window, and 100 distinct
chat avatars. Startup means one initial presentation followed by a queued batch
of 100 asset deltas; it excludes worker scheduling and network time. Individual
asset and switch timings are medians of nine iterations. Switches populate a new
500-row window. These are local measurements, not hardware-independent budgets.

| Measurement | Before | After |
| --- | ---: | ---: |
| Startup fixture full presentations (including the initial one) | 101 | 1 |
| Processing the 100-avatar batch | 643.7 ms | 5.5 ms |
| Single asset: Rust update/presentation | 12.61 ms | 0.06 ms |
| Single asset: remaining layout + software drawing | 124.51 ms | 2.10 ms |
| Chat switch: Rust presentation | 19.00 ms | 7.94 ms |
| Chat switch: remaining layout + software drawing | 148.53 ms | 6.29 ms |

An asset now patches matching consumers directly and needs no full presentation.
The combined asset update/layout/draw cost falls from about 137 ms to 2.2 ms;
a switch falls from about 168 ms to 14.2 ms. The retained models include chat
preview runs, message links, reactions, and rich runs. Asset keys select which
rows and nested runs are updated, including failures and visible profile/header
avatars. Chat cache inputs include text size, order, connection state, metadata,
preview, and draft.

Worker deltas are drained in FIFO order inside one runtime update, followed by
at most one full presentation. Draft edits only capture the editor state and
send the intent. Both draft-only chat publications and identical history-window
publications bypass the full presentation; the draft acknowledgement changes
only its chat row. Send availability remains a local Slint binding. Late
selection drafts cannot replace text already entered in the editor.

Before virtualization, a separate timing of Slint delegate instantiation/change
handlers measured 101.85 ms per asset arrival and 129.08 ms per switch. Chats now
use ListView. History uses ListView when its initial populated window exceeds
60 rows; the mode remains stable while reading that conversation, so arrivals
cannot replace delegates at the cutoff. Small windows keep the existing layout.
The 48-row switch measured 12.57 ms in the layout phase, while an 80-row plain
window measured 22.30 ms. The shared scrollbar, cover offsets, geometry reports,
and deferred pixel-seek correction preserve the existing scroll interface.

Friends and members retain their existing lists: separate 300-row fixtures
measured 0.074 ms and 0.032 ms respectively for delegate/layout processing of an
individual row update, below the 16 ms threshold.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --offline -- -D warnings`
- `cargo clippy -p line-desktop --all-targets --features ui-debug --offline -- -D warnings`
- `cargo test --workspace --offline`
- `cargo build --release --offline -p line-desktop --features ui-debug`
- `python3 tools/check-ui-mcp.py --all --binary target/release/line-desktop`
  runs all behavioural groups, reusing one normal and one wide headless demo
  with fresh synthetic state between groups; signed-out and missing-module
  each have their own launch. Run this command three consecutive times for
  the tooling gate. Each group and the full command report seconds.

For focused iteration, use `--group rich` (or another group), `--wide` for the
wide window, and `--screenshots DIRECTORY` for intermediate and final captures.
The runner builds nothing and uses private temporary XDG directories. MCP
`initialize` retries determine startup readiness, with a 30-second deadline.

No tests were added. Library `src/` was left unchanged. No real account,
external network access, or git commands were used.

## UI check tooling timings (2026-10-19)

The original groups all passed before editing (90.131 seconds). The saved old
runner and launcher were then measured against the final release ui-debug
binary, followed by three consecutive passes of the new `--all` command against
that same unchanged binary. The table uses this second baseline. Seconds include
launch/reset work; the first group in an instance includes startup, and totals
include teardown. The old matrix launched 18 demos; the new matrix launches four.

| Group | Before | After 1 | After 2 | After 3 |
| --- | ---: | ---: | ---: | ---: |
| smoke | 1.287 | 1.055 | 0.855 | 0.876 |
| focus | 12.047 | 13.494 | 8.614 | 8.801 |
| compose | 5.537 | 4.964 | 4.663 | 4.972 |
| scenarios | 6.237 | 1.321 | 1.296 | 1.393 |
| sticker | 3.233 | 1.394 | 1.377 | 1.368 |
| friends | 4.686 | 1.539 | 1.579 | 1.616 |
| layers | 6.238 | 3.318 | 3.280 | 3.907 |
| reading | 2.282 | 1.427 | 1.493 | 2.357 |
| rich | 3.785 | 2.637 | 2.649 | 3.802 |
| media | 3.634 | 0.681 | 0.674 | 0.956 |
| unread | 1.632 | 0.644 | 0.564 | 1.065 |
| detail-stack | included | 2.287 | 2.284 | 3.433 |
| auth-recovery | 2.333 | 1.481 | 1.356 | 2.019 |
| layers-wide | 7.289 | 3.747 | 3.720 | 5.679 |
| focus-wide | 14.851 | 11.880 | 11.338 | 21.913 |
| friends-wide | 5.286 | 1.635 | 1.608 | 1.642 |
| scenarios-wide | 6.487 | 1.448 | 1.379 | 1.352 |
| signed-out | 2.082 | 1.117 | 1.105 | 1.108 |
| missing-module | 0.830 | 0.288 | 0.303 | 0.295 |
| total | 89.756 | 56.363 | 50.144 | 68.566 |

The old `layout-photos` row maps to `sticker`; its viewer checks now run in
`layers`. Detail-stack was included in both friends and layers (also wide); it
now runs once as its own group. The wide matrix retains layers, focus, friends
and scenarios. The merged screenshot option is `--screenshots DIRECTORY`.
Reset uses `reset-demo` through the existing F12 attachment injection hook,
replacing the synthetic backend and in-memory store while retaining the window.

Release build, fmt, workspace all-target clippy, ui-debug all-target clippy
(with warnings denied), offline workspace tests, and the existing demo-dialog
keyboard test passed. Additional checks covered reset after terminal sign-in
failure, attachment arrays, screenshots, and rich content at wide size.
