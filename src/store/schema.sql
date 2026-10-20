CREATE TABLE account(
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    own_mid TEXT NOT NULL DEFAULT '',
    profile_json TEXT NOT NULL DEFAULT '{}',
    sync_revision INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE chat(
    chat_id TEXT PRIMARY KEY,
    is_group INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL DEFAULT '',
    avatar TEXT,
    member_count INTEGER,
    muted INTEGER NOT NULL DEFAULT 0,
    left INTEGER NOT NULL DEFAULT 0,
    last_message_id TEXT,
    last_activity INTEGER,
    server_unread INTEGER NOT NULL DEFAULT 0,
    last_read_server_id TEXT,
    unread_stale INTEGER NOT NULL DEFAULT 0,
    removed INTEGER NOT NULL DEFAULT 0,
    read_acked_server_id TEXT,
    receipt_server_id TEXT
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
    body_json TEXT NOT NULL,
    reactions_json TEXT NOT NULL,
    reactions_rev INTEGER NOT NULL,
    body_rev INTEGER,
    delivery TEXT NOT NULL,
    created_at_local INTEGER NOT NULL,
    mentioned INTEGER NOT NULL DEFAULT 0,
    skip_badge INTEGER NOT NULL DEFAULT 0,
    reply_json TEXT NOT NULL DEFAULT 'null'
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

CREATE TABLE draft(
    chat_id TEXT PRIMARY KEY REFERENCES chat(chat_id),
    text TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    anchor INTEGER NOT NULL DEFAULT 0,
    reply_json TEXT NOT NULL DEFAULT 'null'
);

CREATE TABLE preferences(singleton INTEGER PRIMARY KEY CHECK(singleton=1),value TEXT NOT NULL);

CREATE TABLE contact(mid TEXT PRIMARY KEY,name TEXT NOT NULL,status TEXT NOT NULL,is_group INTEGER NOT NULL,avatar TEXT,member_count INTEGER,original_name TEXT,overridden_name TEXT,relation INTEGER,account_type INTEGER NOT NULL,favorite_time INTEGER, status_metadata TEXT NOT NULL DEFAULT '{}');

CREATE TABLE member(chat_id TEXT NOT NULL REFERENCES chat(chat_id),mid TEXT NOT NULL REFERENCES contact(mid),is_self INTEGER NOT NULL,invited INTEGER NOT NULL,PRIMARY KEY(chat_id,mid));

CREATE TABLE chat_detail(
    chat_id TEXT PRIMARY KEY,
    wire_json TEXT NOT NULL
);

CREATE TABLE sticker_packages(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);

CREATE INDEX message_pending_chat ON message(chat_id,delivery) WHERE server_id IS NULL;

CREATE INDEX message_server_order ON message(chat_id,sort_id) WHERE server_id IS NOT NULL;

CREATE INDEX message_incoming_order ON message(chat_id,sort_id) WHERE outgoing=0 AND server_id IS NOT NULL;

INSERT INTO account(singleton) VALUES(1);
INSERT INTO preferences VALUES(1,'{}');
