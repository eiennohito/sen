//! Flex metadata reduced to readable text and URI actions.
use crate::store::Content;
use serde_json::Value;

pub fn parse(message: &Value) -> Content {
    let metadata = &message["contentMetadata"];
    let source = &metadata["FLEX_JSON"];
    let parsed;
    let source = if let Some(json) = source.as_str() {
        parsed = serde_json::from_str(json).unwrap_or(Value::Null);
        &parsed
    } else {
        source
    };
    let mut lines = Vec::new();
    let mut links = Vec::new();
    visit(source, &mut lines, &mut links);
    let alt_text = metadata["ALT_TEXT"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Flex message")
        .to_owned();
    Content::Flex {
        alt_text,
        lines,
        links,
    }
}

fn visit(node: &Value, lines: &mut Vec<String>, links: &mut Vec<String>) {
    match node["type"].as_str().unwrap_or_default() {
        "bubble" => {
            for section in ["header", "hero", "body", "footer"] {
                visit(&node[section], lines, links);
            }
        }
        "carousel" | "box" => {
            if let Some(contents) = node["contents"].as_array() {
                for child in contents {
                    visit(child, lines, links);
                }
            }
        }
        "text" => {
            if let Some(text) = node["text"].as_str().filter(|s| !s.is_empty()) {
                lines.push(text.into());
            }
        }
        "button" | "image" | "icon" => {}
        _ => return,
    }
    let action = &node["action"];
    if action["type"] == "uri" {
        if let Some(label) = action["label"].as_str().filter(|s| !s.is_empty()) {
            lines.push(label.into());
        }
        if let Some(uri) = action["uri"].as_str().filter(|s| !s.is_empty()) {
            lines.push(uri.into());
            if url::Url::parse(uri).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
                links.push(uri.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flex_bubbles_carousels_and_uri_actions() {
        let module = crate::test_support::installed_module();
        let (mut client, directory) =
            super::super::test_client::client(super::super::Api::new(&module).unwrap());
        let bubble = json!({"type":"bubble", "header":{"type":"text","text":"Title","weight":"bold"},
            "body":{"type":"box","contents":[{"type":"text","text":"First"},
                {"type":"box","contents":[{"type":"text","text":"Nested"}]},
                {"type":"future","text":"Skip","contents":[{"type":"text","text":"Skip too"}]}]},
            "footer":{"type":"box","contents":[
                {"type":"button","action":{"type":"uri","label":"Web","uri":"https://example.org/login"}},
                {"type":"text","text":"Phone","action":{"type":"uri","uri":"line://app/example.org"}},
                {"type":"button","action":{"type":"uri","uri":"http://example.org/help"}}]}});
        for (source, alt, expected) in [
            (
                bubble.clone(),
                Some("Login notice"),
                vec![
                    "Title",
                    "First",
                    "Nested",
                    "Web",
                    "https://example.org/login",
                    "Phone",
                    "line://app/example.org",
                    "http://example.org/help",
                ],
            ),
            (
                json!({"type":"carousel","contents":[bubble,{"type":"bubble","body":{"type":"text","text":"Second bubble"}}]}),
                None,
                vec![
                    "Title",
                    "First",
                    "Nested",
                    "Web",
                    "https://example.org/login",
                    "Phone",
                    "line://app/example.org",
                    "http://example.org/help",
                    "Second bubble",
                ],
            ),
            (Value::Null, Some("Alt only"), vec![]),
        ] {
            // The extension accepts either an encoded string or an object.
            for source in [source.clone(), json!(source.to_string())] {
                assert_eq!(
                    client.message("chat", &json!({"id":"42", "contentType":22,"contentMetadata":{"ALT_TEXT":alt,"FLEX_JSON":source}})).unwrap().unwrap().body,
                    Content::Flex {
                        alt_text: alt.unwrap_or("Flex message").into(),
                        lines: expected.iter().map(|s| (*s).into()).collect(),
                        links: if expected.is_empty() {
                            vec![]
                        } else {
                            vec![
                                "https://example.org/login".into(),
                                "http://example.org/help".into(),
                            ]
                        },
                    }
                );
            }
        }
        drop(client);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
