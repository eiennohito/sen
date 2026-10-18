-- Messages are a server cache; retain only locally created unfinished sends.
DELETE FROM message WHERE server_id IS NOT NULL;
UPDATE message SET body_json=json_object('Text',json_object('text',coalesce(json_extract(body_json,'$.body'),''),'spans',json('[]'),'reply',null)), reactions_json='[]', content_type=0,withdrawn=0;
DELETE FROM segment;
DELETE FROM withdrawal;
DELETE FROM read_marker WHERE state<>'read-acked';
DELETE FROM member;
DELETE FROM contact;
DELETE FROM chat_detail;
UPDATE chat SET kind=(kind<>0),last_key=NULL,last_activity=NULL,unread_stale=1;
ALTER TABLE message ADD COLUMN mentioned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE message ADD COLUMN skip_badge INTEGER NOT NULL DEFAULT 0;
UPDATE account SET profile_json=CASE WHEN profile_json='null' THEN '{}' ELSE profile_json END;
CREATE TABLE preferences(singleton INTEGER PRIMARY KEY CHECK(singleton=1),value TEXT NOT NULL);
INSERT INTO preferences VALUES(1,json_object(
 'text_size',coalesce((SELECT json_extract(value,'$') FROM pref WHERE key='text_size'),15),
 'enter_sends',json(CASE WHEN coalesce((SELECT json_extract(value,'$') FROM pref WHERE key='enter_sends'),0) THEN 'true' ELSE 'false' END),
 'desktop',json(coalesce((SELECT value FROM pref WHERE key='desktop'),'{}')),
 'last_chat',(SELECT json_extract(value,'$') FROM pref WHERE key='last_chat'),
 'request_seq',coalesce((SELECT json_extract(value,'$') FROM pref WHERE key='request_seq'),0)));
DROP TABLE pref;
ALTER TABLE message DROP COLUMN content_type;
ALTER TABLE message DROP COLUMN withdrawn;
ALTER TABLE chat RENAME COLUMN kind TO is_group;

DROP TABLE member;
DROP TABLE contact;
CREATE TABLE contact(mid TEXT PRIMARY KEY,name TEXT NOT NULL,status TEXT NOT NULL,is_group INTEGER NOT NULL,avatar TEXT,member_count INTEGER,original_name TEXT,overridden_name TEXT,relation INTEGER,account_type INTEGER NOT NULL,favorite_time INTEGER);
CREATE TABLE member(chat_id TEXT NOT NULL REFERENCES chat(chat_id),mid TEXT NOT NULL REFERENCES contact(mid),is_self INTEGER NOT NULL,invited INTEGER NOT NULL,PRIMARY KEY(chat_id,mid));
