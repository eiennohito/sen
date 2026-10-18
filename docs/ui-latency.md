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
- `cargo clippy --workspace --all-targets --features ui-debug --offline -- -D warnings`
- `cargo test --workspace --offline`
- `cargo build --release --offline -p line-desktop --features ui-debug`
- Fresh owned headless instances for every `check-ui-mcp.py` group: default,
  unread, scenarios, friends, layout-photos, reading, rich, media, auth-recovery,
  layers, focus; also wide instances for layers, focus, friends, scenarios.

Additional standalone scripts were investigated separately. The saved pre-change
binary reproduces the same browser native-context-menu assertion, composer
Alt+Down/profile assertion, and unreachable “Group members” fixture assertion.
The search script's launcher supplies `--demo-text-size` and
`--demo-color-scheme`, which this checkout does not accept. These pre-existing
script failures are outside the latency changes; the requested check-ui groups
are the UI acceptance checks above.

No tests were added. Library `src/` was left unchanged. No real account,
external network access, or git commands were used.
