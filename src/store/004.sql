ALTER TABLE message ADD COLUMN reply_json TEXT NOT NULL DEFAULT 'null';
UPDATE message SET reply_json=coalesce(json_extract(body_json,'$.Text.reply'),'null'), body_json=json_remove(body_json,'$.Text.reply');
