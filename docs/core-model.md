# Core model: store, worker, projection

Status: accepted 2026-10-18 (full rewrite, SQLite, FakeBackend demo, no legacy import). Replaces the PoC message model (D0)
and the worker/controller artifacts found in the 2026-10-18 audits. Behaviour
stays [ux-model.md](ux-model.md); engineering constraints not overridden here
stay in [app-design.md](app-design.md). Nothing in this document is
implemented yet.

## Why a rewrite, not fixes

The PoC keeps up to seven unowned copies of a chat's messages (worker page
files and memo, `View.messages`, `history[chat]`, `recent[chat]`, the outbox,
`aliases`), and each source assigns a whole list, so the last arrival wins
(M13, M14). Around it, the main flows are held together by mechanisms that
lose work silently:

| Artifact | Failure |
| --- | --- |
| Worker `pending: Option<Command>` slot (`account.rs:580…804`) | Any push overwrites the post-send refresh, `ChatDetails` or `Receipts`. |
| `pending = Chats` after every live operation | Three RPCs per operation; resets chat-list pagination (`desktop.rs:290`). |
| `connect()` drops unknown commands (`account.rs:1121`) | A chat opened while offline stays "loading" forever. |
| `Client::open` requires the network; every failure becomes `ConnectionFailure::Session` | Offline start shows "Signed out" and offers re-pairing. |
| Deferred updates while the pointer is down (`deferred.rs`) | Drops and reorders updates; `[Messages, Incoming, Messages]` loses the incoming row. |
| `Update::Chats` assigns preview/activity unconditionally, never removes rows | Stale responses regress previews; left chats stay forever. |
| Three different ordering tiebreaks (`view.rs:675,1268`, `paging.rs:5`) | Same-millisecond messages reorder between paths. |
| Outbox: string statuses, no discard, 256 cap | Failed items accumulate until Send is disabled. |
| SSE thread exits on credential/IO errors; pump threads exit after one 2 s UI stall | Live sync, tray and notification actions die until restart. |
| All worker errors reduced to `Err(_)` or UI strings | Auth expiry, offline, server and decode failures are indistinguishable. |
| `start_demo` reimplements the worker protocol; 66 `demo` branches in production code | The demo cannot reproduce M13/M14 because it skips `load()`. |
| `View`: ~55 fields, a 1,050-line `update` match, Slint properties read as state | Every flow above is entangled in it. |

None of these is fixed by patching one site, because the defects come from
having no owner and no typed state. The rewrite below gives each piece of
state exactly one owner.

## Ownership

```
 UI thread                         worker thread                    other threads
 ─────────                         ─────────────                    ─────────────
 Controller (AppState)  ──Intent──▶ Scheduler ──▶ Engine ──▶ Store (SQLite, sole writer)
   │  ▲                                  ▲          │
   │  └──────── Delta (FIFO) ────────────┼──────────┘
   ▼                                     │
 Presenter → Slint models          Sync supervisor ◀── SSE thread (ops)
                                   Asset fetchers (unchanged)
```

- **Store** is the only source of truth for messages, chats, contacts,
  members, drafts, the outbox, unread state, preferences and the sync
  revision. Only the worker thread opens it for writing.
- **Engine** (worker thread) turns every input into store transactions:
  server pages, older/newer pages, live operations, send results, echoes,
  unsends, reactions, receipts. After each transaction it emits deltas for what
  the UI currently watches.
- **Controller** (UI thread) holds `AppState`: plain Rust data, a *projection*
  of the store (the chat list, plus a window of the open chat), and pure UI
  state (selection, panels, focus intents, composer text). It changes the
  projection only by applying deltas. It never assigns message lists from
  responses.
- **Presenter** is the only code that touches Slint. It diffs `AppState`
  into keyed `VecModel`s (chats, messages, friends, members) and sets
  properties.

## Store

SQLite through `rusqlite` with the `bundled` feature (no system library, one C
compile). One database per account at
`$XDG_DATA_HOME/line-desktop/accounts/<own-mid-hash>/store.sqlite3`, in a
directory with mode 0700, WAL mode, `synchronous=NORMAL`. It is not encrypted
(user decision 2026-10-18). Migrations are numbered SQL scripts recorded in
`PRAGMA user_version`.

The 2026-10-15 rule in app-design.md, "no database to draw the window", is
narrowed: the UI never queries the database and never waits on it. Drawing
depends only on `AppState`.

Tables (columns abbreviated):

- `account(own_mid, profile_json, sync_revision, global_revision,
  individual_revision, opened_at)`. One row.
- `chat(chat_id PK, kind, title, avatar, member_count, muted, left,
  last_key, last_activity, server_unread, last_read_server_id,
  unread_stale, details_at, list_rank)`.
- `message(key INTEGER PK, chat_id, server_id UNIQUE NULL, local_id UNIQUE
  NULL, request_seq NULL, sort_ts, sort_id, sender_id, outgoing,
  content_type, body_json, reactions_json, reactions_rev, withdrawn,
  delivery, delivery_detail, created_at_local)`, with an index
  `(chat_id, sort_ts, sort_id)`.
- `segment(chat_id, lo_ts, lo_id, hi_ts, hi_id, reaches_start)`. These are the
  contiguous ranges known to have no gaps (see "Gaps").
- `contact`, `member(chat_id, mid, …)`, `draft(chat_id, text, cursor,
  updated)`, `pref(key, value)`, `read_marker(chat_id, server_id, state)`.

`key` is the row identity for the life of the row. It does not change when a
pending send gets its server id. The UI keys every row by `key` and never by
server or local id, which removes `aliases`, `canonical_history_id` and
`displayed_history_id`.

### Order

There is one total order everywhere: `(sort_ts, sort_id)`.

- Confirmed rows: `sort_ts = createdTime` (falling back to `deliveredTime`),
  and `sort_id` is the server id as a zero-padded fixed-width decimal.
- Pending rows: `sort_ts = local send time`, and `sort_id = "~" + local_id`,
  so a pending row sorts after any confirmed row with the same millisecond.
- On confirmation, `sort_ts`/`sort_id` take the server values. The row keeps
  its `key` and may move by the clock difference.

### Merge rules (one function per input, all inside one transaction)

| Input | Rule |
| --- | --- |
| Server page (recent, older, newer) | Upsert by `server_id`. Content fields take the newer `reactions_rev`/edit; `withdrawn` is sticky; `delivery` only advances (Sent → Read). Never deletes rows. Extends or merges `segment`s. |
| Live message op (25/26/43) | Upsert by `server_id`. Outgoing op 25 with a `request_seq` matching a pending row links to that row (sets `server_id`, `delivery=Sent`). Adjacent to the newest segment when the stream is continuous since the last applied revision; otherwise starts a new segment. |
| Send result | Link the pending row by `request_seq` → `server_id`. If the echo already created a separate row, merge the two (keep the pending row's `key`, delete the duplicate, emit `Removed{key}`). |
| Send failure | `delivery = Rejected{reason}` or `Uncertain`. The row stays until resolved. |
| Unsend op 64/65 | `withdrawn = true`, body cleared, `delivery` untouched. |
| Reaction op 139/140 | Replace `reactions_json` if the op's revision is newer. |
| Receipt op 55 / read ranges | Advance `delivery` to Read for outgoing rows up to the marker. |
| Read elsewhere op 40 | Advance `chat.last_read_server_id`; recompute unread. |
| Chat list page | Upsert `chat` rows. Preview and activity come from the chat's newest message row, never from the list payload, unless the list's last message is newer than anything stored. In that case insert that message row, which also marks the chat as needing a refresh (M14). Chats absent from a *complete* list sweep are marked `left`. |

Preview, activity, unread and history are all read from the same rows, so
they cannot disagree.

### Gaps

A chat's stored rows are a union of contiguous segments. On open:

1. The engine sends the window around the anchor from the store at once,
   offline or not.
2. If the newest stored row is older than `chat.last_key`, or the newest
   segment is not continuous with live sync, the engine fetches recent
   messages. It merges them, and joins segments when the page overlaps one.
   If the page does not overlap, a gap remains: the window contains a "gap"
   marker row that the UI turns into an automatic newer/older fetch when it
   becomes visible.

Messages are never deleted for size. Media stays a bounded LRU cache with no
age expiry (D8).

### Outbox

Pending sends are `message` rows whose `delivery` is one of `Pending`,
`Rejected{reason}`, `Uncertain`, `Sent`, `Read` (a Rust enum serialised as
JSON, never compared as a string). The submit flow is:

1. Insert a pending row (a durable commit on the worker thread; the UI waits
   only for the `Inserted` delta, not for the disk).
2. Send it.
3. Link it, or mark it failed.

The actions on failed rows are Retry (same row, new `request_seq`), Discard
(deletes the row) and Copy to composer. Resend after `Uncertain` creates a new
row and discards the old one after a duplicate-risk confirmation. There is no
count cap; a chat's failed rows are listed in its row badge until resolved.

### Legacy import

Nothing is imported (user decision 2026-10-18). On the first open the store
starts empty; the old `desktop.state`, encrypted message pages and
`protocol.state` are deleted, and the encryption code is removed (D7, D6).

## Worker

### Scheduler (replaces `pending` and the shared 8-slot queue)

The worker has one `Inbox` (a `Mutex` plus `Condvar`) with three lanes, served
in this order:

1. **User intents**: open chat, send, older/newer, mark read, retry, discard,
   friends, members, sign-in. FIFO and unbounded, because the UI produces them
   at human rate. They are never dropped, so "Account is busy" disappears.
2. **Live operations** from the SSE thread: FIFO, bounded at 256. The SSE
   thread blocks when the lane is full (backpressure, no drops).
3. **Refresh intents**: a *set* (`ChatList`, `ChatDetails(chat)`,
   `Recent(chat)`, `Receipts(chat)`, `Friends`, `Members(chat)`). Inserting an
   intent that is already present is a no-op, so nothing overwrites anything.
   Each intent records its reason only for logging.

A live operation applies its delta directly (move the chat up, write the row,
bump unread). It only schedules `ChatDetails(chat)` when the chat is unknown
or its metadata op changed. The chat-list pagination cursor belongs to the
list loader, not to refresh.

### Connection state machine

```rust
enum Connection {
    Local,                         // store open, no network attempt yet
    Connecting { attempt: u32 },
    Online,
    Offline { retry_at: Instant, cause: NetCause },
    AuthExpired,                   // refresh failed; needs sign-in
    SessionInvalid,                // MID mismatch / revoked
    SigningIn { stage: PairStage },
}
```

The worker owns it and publishes each transition as one delta. The UI derives
the banner and the enabled actions from it with a pure function. This replaces
the nine booleans.

Startup is two-phase:

- **Local open** needs no network: credentials, store, WASM. It immediately
  publishes the chat list, the account name, the last open chat and that chat's
  window, with unread counts marked stale (ux-model §150–155).
- **Online open** validates the profile, then resumes sync from the stored
  revision. A MID mismatch moves to `SessionInvalid` and leaves the store
  untouched.

Intents that need the network while not `Online` are kept as refresh intents
and run on the transition to `Online`. They are not dropped.

### Typed errors

`client::Error { Offline(NetCause), Auth(AuthCause), Server{code},
Rejected{code}, Decode, Crypto, Store }`, built with `thiserror` at the
`Client` boundary. The engine maps errors to connection transitions and
per-row states. User-facing text is produced only in the presenter.

### Sync supervisor

The SSE thread is restarted with backoff on every non-terminal exit. Terminal
states are only `AuthExpired`, `SessionInvalid` and shutdown. The engine
persists the revision in the same transaction that applies an operation, and
seeds `localRev` from it on start (D5). Operations replayed during catch-up
carry `replay=true`, which suppresses notifications and unread increments. The
UI-thread pumps (tray, notifications, activation) become channels drained on
the event loop. They never exit on a slow turn.

### Backend seam

The engine talks to a `Backend` trait at the RPC level: `profile`, `chats`,
`messages page`, `send`, `ops stream`, `media`. It has two implementations:

- `LineBackend`: the existing transport, WASM and crypto.
- `FakeBackend`: in-process, scripted. It serves canned chats and messages and
  can inject operations, delays and failures.

The demo (`--demo`), `--snapshot` scenes and integration tests run the *real*
engine, store and controller on `FakeBackend`. `start_demo` and the `demo`
branches in `View`, `paging` and `main` are deleted.

## Projection and deltas

The UI subscribes to `ChatList` (sorted rows, bounded to what the list shows
plus a margin) and `Window { chat, anchor, before, after }`. The worker
answers with a snapshot, then pushes deltas in commit order over a single FIFO
(`upgrade_in_event_loop` preserves order):

```rust
enum Delta {
    Connection(Connection),
    Chats { upsert: Vec<ChatItem>, remove: Vec<ChatId> },
    Window { sub: SubId, upsert: Vec<MessageItem>, remove: Vec<Key>,
             gap_markers: Vec<Gap>, reaches_start: bool },
    Unread { chat: ChatId, count: u32, stale: bool, divider: Option<Key> },
    …
}
```

- A `sub` id fences deltas for a window that was replaced by a newer
  subscription. This replaces the `generation` checks.
- The controller applies deltas immediately, including while the pointer is
  down. The presenter preserves row identity by `key`, so selection and menus
  survive patches. This deletes `deferred.rs`. If pointer-down stability needs
  more than keyed patching, the presenter (not the data layer) may hold back
  *model commits* until release, without dropping anything.
- Scrolling far moves the window: the controller asks for a new window
  anchored at the visible row. The store is the history, so there is no UI-side
  retention policy and no `RETAINED_ROWS`.
- Image bitmaps are not stored in rows. The presenter looks them up by asset
  key in the existing decoded-image cache when it builds a row.

## Tests that must exist

Each of the following runs against `FakeBackend` through the real engine,
store and controller:

- M13: send, switch chat, return, with a stale page arriving late; the row is
  kept with the same `key`.
- M14: an incoming message in a closed chat, then opening that chat with a
  stale stored page; the row is shown and a refresh is scheduled.
- Echo before send result, and send result before echo: one row in both cases.
- Offline start shows cached chats and the cached window; reconnect resumes
  from the stored revision with no duplicate notifications.
- Chat-list pagination survives live operations.
- Updates during pointer-down are all applied, none lost.
- Unsend, reaction and receipt each update the row in place.
- Failed send: discard, retry, resend-after-uncertain.
- Restart: history, drafts, failed outbox items and the scroll anchor are
  restored from the store.

## Implementation phases

Each phase is one Codex task in its own worktree, reviewed before merge.
`main` stays buildable and the app usable after each phase.

1. **Store library** (`src/store/`): schema, migrations, merge functions,
   order, segments, outbox states, legacy import. Pure unit tests, no UI.
2. **Engine and backend seam**: `Backend` trait, `LineBackend` (moved from
   `Client`), `FakeBackend`, scheduler, connection state machine, two-phase
   open, sync supervisor with persisted revision, typed errors. The current
   `Update` protocol is kept temporarily through an adapter so the old `View`
   still runs.
3. **Controller and presenter**: `AppState`, deltas, keyed models. Delete
   `history`, `recent`, `aliases`, `deferred.rs`, the outbox in `Saved`, the
   demo branches and `start_demo`. Snapshot scenes move to `FakeBackend`
   scripts. Probably split into 3a (chat list and window) and 3b (outbox,
   unread, connection banner).
4. **Cleanup**: remove the legacy encryption and the old cache, media cache
   LRU without expiry (D8), update app-design.md and ux-acceptance rows, reset
   V6 rows.

Out of scope here: U-group surfaces, P-group protocol additions (the engine
gets a slot for reaction and unsend ops, which T5/P7 then fill), and T9.
