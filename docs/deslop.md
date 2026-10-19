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

Add a row when a pass finds a new pattern.
