CREATE TABLE account(
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    own_mid TEXT NOT NULL DEFAULT '',
    profile_json TEXT NOT NULL DEFAULT 'null',
    sync_revision INTEGER NOT NULL DEFAULT 0,
    global_revision INTEGER NOT NULL DEFAULT 0,
    individual_revision INTEGER NOT NULL DEFAULT 0,
    opened_at INTEGER NOT NULL DEFAULT (unixepoch()*1000)
);

INSERT INTO account(singleton) VALUES(1);

CREATE TABLE chat(
    chat_id TEXT PRIMARY KEY,
    kind INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL DEFAULT '',
    avatar TEXT,
    member_count INTEGER,
    muted INTEGER NOT NULL DEFAULT 0,
    left INTEGER NOT NULL DEFAULT 0,
    last_key TEXT,
    last_activity INTEGER,
    server_unread INTEGER NOT NULL DEFAULT 0,
    last_read_server_id TEXT,
    unread_stale INTEGER NOT NULL DEFAULT 0,
    details_at INTEGER,
    list_rank INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE message(
    key INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id TEXT NOT NULL REFERENCES chat(chat_id),
    server_id TEXT UNIQUE,
    local_id TEXT UNIQUE,
    request_seq INTEGER,
    sort_ts INTEGER NOT NULL,
    sort_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    outgoing INTEGER NOT NULL,
    content_type INTEGER NOT NULL,
    body_json TEXT NOT NULL,
    reactions_json TEXT NOT NULL,
    reactions_rev INTEGER NOT NULL,
    content_rev INTEGER,
    withdrawn INTEGER NOT NULL,
    delivery TEXT NOT NULL,
    delivery_detail TEXT,
    created_at_local INTEGER NOT NULL
);

CREATE INDEX message_order ON message(chat_id,sort_ts,sort_id);

CREATE UNIQUE INDEX message_pending_request ON message(request_seq) WHERE server_id IS NULL AND request_seq IS NOT NULL;

CREATE TABLE segment(
    chat_id TEXT NOT NULL REFERENCES chat(chat_id),
    lo_ts INTEGER NOT NULL,
    lo_id TEXT NOT NULL,
    hi_ts INTEGER NOT NULL,
    hi_id TEXT NOT NULL,
    reaches_start INTEGER NOT NULL
);

CREATE INDEX segment_chat ON segment(chat_id,lo_ts,lo_id);

CREATE TABLE withdrawal(
    server_id TEXT PRIMARY KEY
);

CREATE TABLE contact(
    mid TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL
);

CREATE TABLE member(
    chat_id TEXT NOT NULL REFERENCES chat(chat_id),
    mid TEXT NOT NULL,
    profile_json TEXT NOT NULL,
    PRIMARY KEY(chat_id,mid)
);

CREATE TABLE draft(
    chat_id TEXT PRIMARY KEY REFERENCES chat(chat_id),
    text TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    updated INTEGER NOT NULL
);

CREATE TABLE pref(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE read_marker(
    chat_id TEXT NOT NULL REFERENCES chat(chat_id),
    server_id TEXT NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY(chat_id,state)
);
