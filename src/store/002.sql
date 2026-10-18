-- Phase 6, schema 3: schema 2 already added the draft selection anchor.
CREATE TABLE IF NOT EXISTS chat_detail (chat_id TEXT PRIMARY KEY, wire_json TEXT NOT NULL);
ALTER TABLE chat ADD COLUMN removed INTEGER NOT NULL DEFAULT 0;
UPDATE chat SET removed=left;
UPDATE contact SET profile_json=json_set(profile_json,'$.profile',json('{"relation":0}')) WHERE json_extract(profile_json,'$.profile') IS NULL AND json_extract(profile_json,'$.group')=0;
