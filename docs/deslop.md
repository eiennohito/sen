# Deslop

Every LLM session leaves some slop. It is removed on a schedule, not when
someone trips over it.

## Cadence

- After every feature phase merges: one deslop pass before the next feature
  starts. A read-only audit lists findings against the patterns below;
  a cleanup session deletes or replaces them; a review checks nothing broke.
- Each feature brief links this file. New code must not add these patterns,
  and reviews flag them.
- A deslop pass replaces, it does not patch: the bad thing is deleted and its
  callers changed in the same commit (see "No compatibility code" in
  [app-design.md](app-design.md)).

## Review stance

Assume the session produced something that only looks like a program.

- A session's own report ("fmt, clippy, tests, checks passed") proves
  nothing about the design. The coordinator re-runs checks; the review judges
  the code.
- Every new type, flag, layer, fallback, option, test and comment must
  justify itself: why does it exist, what breaks if it is deleted?
- Compare against what a competent engineer would have written, not against
  the session's intent. Far from it means rewrite, not polish.
- Hunt for imitation of work: tests asserting the implementation, unneeded
  validation and logging, single-use generic abstractions, robustness for
  impossible states, docs describing intent instead of code.
- Verdicts are blunt: "delete", "rewrite as X", "wrong model".

## Rare cases

Sessions invent rare cases and give each one its own state, flag or code
path. Triage instead:

1. Make it self-healing so it doesn't matter in practice.
2. Fold it into the existing model in a better way, without a new state.
3. Accept it as a degradation if it isn't critical, and say so in one line.

Example: a send that may have reached LINE before the connection dropped is
just "Not sent" with Retry (2); our echo arriving later turns any row into
Sent (1). No "may not have been sent" state.

## Tests are code under review

Passing tests show only that tests and code agree. Tests are audited like
product code, and every deslop pass covers them.

Keep a test only if it pins something a user or the protocol would notice:
a user-visible flow, a protocol or crypto fact (fixtures, byte-exact
vectors), a data-loss or double-send regression, a past real bug.

Delete, don't fix:
- tests that restate the implementation (same formula, same branches, the
  mock returning what the assertion expects);
- tests of private helpers already covered by a flow test;
- tests for impossible states, and corner cases nobody hits;
- several tests for one behaviour where one would do;
- sleep- or timing-based tests (wait on events);
- UI checks that assert layout numbers or element counts instead of what the
  user can see and do;
- fake backends growing knobs only one test uses.

Before deleting a test, check it isn't the only coverage of something a
user would notice: audit lists get this wrong too (a cleanup removed the
only tests of contact/group live operations and of pairing after a broken
saved session). A replaced or new test must fail when the behaviour is
broken; break it once and see.

A whole-suite audit runs with the next deslop pass. Suite runtime is part
of the budget: slow tests cost development speed.

## Patterns seen in this codebase

| Pattern | Example that was found |
|---|---|
| Compatibility shims before any release: aliases, re-exports, forwarding `<=>`, "preserves the old API" | 85 `AppWindow` properties forwarding `State` |
| Config or files that only point at a dev checkout | `paths.json` redirecting to `artifacts/` |
| PoC names and leftovers next to their replacements | crate `line-wasm-poc`, PoC binary, "not used by this PoC" |
| God objects that every feature appends to | `State` global: 213 properties, 100 callbacks |
| State as a pile of flags instead of one explicit state | `has_session`, `session_known`, `connection` and generation checks deciding sign-in |
| Silent catch-alls: `_ => return`, `let _ = send(..)`, infinite silent retry | net jobs dropped into a dead channel; unknown server errors retried forever |
| Ad-hoc error matching instead of one classification point | HTTP 401/403 as the only auth signal |
| Rebuilding long-lived objects per call | WASM group key and channel rebuilt per send; signing key per request |
| Two code paths for one job | send and receive using different channel handling |
| Decisions in the view layer | sign-in availability computed in `.slint` expressions |
| Generated-code bloat | 10 MB of Rust from 3k lines of `.slint` |
| Demo or debug code in the product build | demo scenario dialog compiled into the app |
| Pinning what upstream changes | fixed module path, hash and version of the Chrome extension |
| Over-validation and corner-case tests that slow development | poison recovery on 78 mutex unwraps |
| Misleading names | `paths.json`, a "local data could not be saved" message for an auth failure |
| Comments that narrate history instead of describing the code | "old square cache entries…", "legacy" wording |
| Changing a persisted format without loading what the previous code wrote | typed `Session` renamed fields and would have signed the user out; a split error enum made stored failed sends unreadable |
| "Consolidation" that moves code instead of removing it, and quietly changes wording | five label tables became one plus per-call-site variants; unnamed groups shown as "Unknown member" |

Persisted data has two rules. The saved session survives every change:
any change to its type comes with a test that loads JSON in the exact
shape the previous code wrote and saves it back without losing a field.
The local store may be discarded (bump the schema version), but never
left half-readable.

Add a row when a pass finds a new pattern.
