//! Typed, optional message details. Malformed metadata never discards the body.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichContent {
    pub content_type: i64,
    #[serde(default)]
    pub album_count: Option<u64>,
    #[serde(default)]
    pub caption: Option<String>,
    pub emoji: Vec<InlineEmoji>,
    pub reply: Option<ReplyReference>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    /// LINE's DURATION metadata is in milliseconds.
    pub duration_ms: Option<u64>,
    pub contact_name: Option<String>,
    pub system_text: Option<String>,
    /// Localization data is preserved even when no literal system text exists.
    pub system_key: Option<String>,
    pub system_args: Option<String>,
    pub withdrawn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyReference {
    pub message_id: String,
    pub service_code: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineEmoji {
    /// Half-open JavaScript UTF-16 offsets, never inside a surrogate pair.
    pub start_utf16: usize,
    pub end_utf16: usize,
    /// Equivalent UTF-8 boundaries for slicing the unchanged message body.
    pub start_byte: usize,
    pub end_byte: usize,
    pub fallback: String,
    pub product_id: String,
    pub sticon_id: String,
    pub version: Option<String>,
    pub animated: bool,
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn nonempty(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}
fn utf16_boundary(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}

pub fn parse(raw: &Value) -> RichContent {
    let metadata = &raw["contentMetadata"];
    let content_type = raw["contentType"].as_i64().unwrap_or_default();
    let text = raw["text"].as_str().unwrap_or_default();
    let mut details = RichContent {
        content_type,
        album_count: matches!(content_type, 1 | 2)
            .then(|| number(&metadata["GTOTAL"]))
            .flatten()
            .filter(|n| *n > 1),
        caption: matches!(content_type, 1 | 2)
            .then(|| nonempty(&raw["text"]))
            .flatten(),
        reply: (raw["messageRelationType"].as_i64() == Some(3))
            .then(|| nonempty(&raw["relatedMessageId"]))
            .flatten()
            .map(|message_id| ReplyReference {
                message_id,
                service_code: raw["relatedMessageServiceCode"].as_i64(),
            }),
        withdrawn: raw["isUnsent"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    match content_type {
        2 | 3 => details.duration_ms = number(&metadata["DURATION"]),
        13 => details.contact_name = nonempty(&metadata["displayName"]),
        14 => {
            details.file_name = nonempty(&metadata["FILE_NAME"]);
            details.file_size = number(&metadata["FILE_SIZE"]);
        }
        18 => {
            details.system_text = nonempty(&raw["text"]);
            details.system_key = nonempty(&metadata["LOC_KEY"]);
            details.system_args = nonempty(&metadata["LOC_ARGS"]);
        }
        _ => {}
    }
    // REPLACE is a JSON string in both plain and E2EE-decoded messages.
    let replace = if metadata["REPLACE"].is_object() {
        Some(metadata["REPLACE"].clone())
    } else {
        metadata["REPLACE"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
    };
    if let Some(resources) = replace
        .as_ref()
        .and_then(|v| v["sticon"]["resources"].as_array())
    {
        for resource in resources {
            let span = (|| {
                let start = usize::try_from(number(&resource["S"])?).ok()?;
                let end = usize::try_from(number(&resource["E"])?).ok()?;
                if start >= end {
                    return None;
                }
                let start_byte = utf16_boundary(text, start)?;
                let end_byte = utf16_boundary(text, end)?;
                Some(InlineEmoji {
                    start_utf16: start,
                    end_utf16: end,
                    start_byte,
                    end_byte,
                    fallback: text[start_byte..end_byte].to_owned(),
                    product_id: nonempty(&resource["productId"])?,
                    sticon_id: nonempty(&resource["sticonId"])?,
                    version: nonempty(&resource["version"]),
                    animated: resource["resourceType"].as_str() == Some("ANIMATION"),
                })
            })();
            if let Some(span) = span {
                details.emoji.push(span);
            }
        }
        details
            .emoji
            .sort_by_key(|span| (span.start_utf16, span.end_utf16));
        let mut end = 0;
        details.emoji.retain(|span| {
            if span.start_utf16 < end {
                return false;
            }
            end = span.end_utf16;
            true
        });
    }
    details
}

/// A group map contains current members including self; legacy room lists
/// exclude self. Missing membership data remains unknown, not zero.
pub fn member_mids(chat: &Value, my_mid: &str) -> Option<Vec<String>> {
    let group = chat
        .pointer("/extra/groupExtra/memberMids")
        .and_then(Value::as_object);
    let mut ids: HashSet<&str> = if let Some(members) = group {
        members.keys().map(String::as_str).collect()
    } else {
        chat["memberMids"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect()
    };
    ids.remove("");
    if group.is_none() && !my_mid.is_empty() {
        ids.insert(my_mid);
    }
    let mut ids: Vec<_> = ids.into_iter().map(str::to_owned).collect();
    ids.sort();
    Some(ids)
}

pub fn group_member_count(chat: &Value, my_mid: &str) -> Option<usize> {
    if let Some(members) = chat
        .pointer("/extra/groupExtra/memberMids")
        .and_then(Value::as_object)
    {
        return Some(members.len());
    }
    member_mids(chat, my_mid).map(|ids| ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn group_map_count_does_not_invent_self() {
        let chat = json!({"extra":{"groupExtra":{"memberMids":{"other":"1"}}}});
        assert_eq!(group_member_count(&chat, "self"), Some(1));
        assert_eq!(member_mids(&chat, "self").unwrap(), ["other"]);
        assert_eq!(
            group_member_count(&json!({"extra":{"groupExtra":{"memberMids":{}}}}), "self"),
            Some(0)
        );
    }

    #[test]
    fn extracts_members_without_invitees_or_duplicates() {
        assert_eq!(member_mids(&json!({}), "self"), None);
        for raw in [
            json!({"extra":{"groupExtra":{"memberMids":{"self":"1","other":"2"},"inviteeMids":{"invited":"3"}}}}),
            json!({"memberMids":["other","self","other","",null]}),
            json!({"memberMids":["other"]}),
        ] {
            assert_eq!(member_mids(&raw, "self").unwrap(), ["other", "self"]);
        }
        assert_eq!(
            member_mids(&json!({"memberMids":[]}), "").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn emoji_offsets_are_utf16_and_preserve_japanese_astral_fallback() {
        let raw = json!({"text":"日😀[笑]本🌸", "contentMetadata":{"REPLACE":json!({"sticon":{"resources":[
            {"S":3,"E":6,"productId":"p","sticonId":"s","resourceType":"ANIMATION"},
            {"S":"7","E":"9","productId":"p","sticonId":"flower"}
        ]}}).to_string()}});
        let details = parse(&raw);
        assert_eq!(details.emoji.len(), 2);
        let a = &details.emoji[0];
        assert_eq!(
            (a.start_byte, a.end_byte, a.fallback.as_str()),
            (7, 12, "[笑]")
        );
        assert!(a.animated);
        assert_eq!(details.emoji[1].fallback, "🌸");
        assert_eq!(
            &raw["text"].as_str().unwrap()[details.emoji[1].start_byte..details.emoji[1].end_byte],
            "🌸"
        );
    }

    #[test]
    fn malformed_and_overlapping_spans_do_not_hide_valid_content() {
        let mut raw = json!({"text":"日😀abc", "contentMetadata":{"REPLACE":json!({"sticon":{"resources":[
            {"S":2,"E":3,"productId":"p","sticonId":"split"},
            {"S":3,"E":5,"productId":"p","sticonId":"valid"},
            {"S":4,"E":6,"productId":"p","sticonId":"overlap"},
            {"S":0,"E":999999999,"productId":"p","sticonId":"outside"},
            {"S":-1,"E":1,"productId":"p","sticonId":"negative"},
            {"S":0,"E":1}, {"S":0,"E":0,"productId":"p","sticonId":"empty"}
        ]}}).to_string()}});
        assert_eq!(
            parse(&raw)
                .emoji
                .iter()
                .map(|v| v.fallback.as_str())
                .collect::<Vec<_>>(),
            ["ab"]
        );
        raw["contentMetadata"]["REPLACE"] = json!("{bad json");
        assert!(parse(&raw).emoji.is_empty());
        assert_eq!(raw["text"], "日😀abc");
    }

    #[test]
    fn typed_optional_details_reject_bad_numbers_and_non_reply_relations() {
        let mut raw = json!({"contentType":14,"messageRelationType":3,"relatedMessageId":"reply-id","relatedMessageServiceCode":1,
            "contentMetadata":{"FILE_NAME":"資料.txt","FILE_SIZE":"18446744073709551615"}});
        let details = parse(&raw);
        assert_eq!(details.file_name.as_deref(), Some("資料.txt"));
        assert_eq!(details.file_size, Some(u64::MAX));
        assert_eq!(details.reply.unwrap().message_id, "reply-id");
        raw["contentMetadata"]["FILE_SIZE"] = json!("-1");
        raw["messageRelationType"] = json!(0);
        assert!(parse(&raw).file_size.is_none());
        assert!(parse(&raw).reply.is_none());
        for kind in [2, 3] {
            assert_eq!(
                parse(&json!({"contentType":kind,"contentMetadata":{"DURATION":"1250"}}))
                    .duration_ms,
                Some(1250)
            );
            assert!(
                parse(&json!({"contentType":kind,"contentMetadata":{"DURATION":"NaN"}}))
                    .duration_ms
                    .is_none()
            );
        }
        assert_eq!(
            parse(&json!({"contentType":13,"contentMetadata":{"displayName":"葵"}}))
                .contact_name
                .as_deref(),
            Some("葵")
        );
        let system = parse(
            &json!({"contentType":18,"text":"Synthetic system item","contentMetadata":{"LOC_KEY":"C_J","LOC_ARGS":"synthetic"},"isUnsent":true}),
        );
        assert!(system.withdrawn);
        assert_eq!(system.system_text.as_deref(), Some("Synthetic system item"));
        assert_eq!(system.system_key.as_deref(), Some("C_J"));
    }

    #[test]
    fn album_count_and_caption_are_optional_media_details() {
        let rich = parse(
            &json!({"contentType":1,"text":"Riverside", "contentMetadata":{"GID":"group", "GTOTAL":"4"}}),
        );
        assert_eq!(rich.album_count, Some(4));
        assert_eq!(rich.caption.as_deref(), Some("Riverside"));
        for total in ["0", "1", "bad", "-1"] {
            assert!(
                parse(&json!({"contentType":2,"contentMetadata":{"GTOTAL":total}}))
                    .album_count
                    .is_none()
            );
        }
        assert!(
            parse(&json!({"contentType":0,"text":"plain","contentMetadata":{"GTOTAL":"4"}}))
                .caption
                .is_none()
        );
    }

    #[test]
    fn membership_is_unknown_when_absent_and_counts_self_once() {
        assert_eq!(group_member_count(&json!({}), "self"), None);
        assert_eq!(
            group_member_count(
                &json!({"extra":{"groupExtra":{"memberMids":{"self":"1","other":"2"},"inviteeMids":{"invited":"3"}}}}),
                "self"
            ),
            Some(2)
        );
        assert_eq!(
            group_member_count(&json!({"memberMids":["other","other"]}), "self"),
            Some(2)
        );
    }
}
