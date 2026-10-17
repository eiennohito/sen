use super::*;

fn message(id: &str, text: &str) -> account::MessageView {
    account::MessageView {
        id: id.into(),
        sender_id: String::new(),
        sender_name: String::new(),
        sender_key: String::new(),
        sticker: false,
        photo: false,
        body: text.into(),
        timestamp: 1,
        delivered_time: 1,
        rich: Default::default(),
        outgoing: false,
        image_key: String::new(),
        reactions: Vec::new(),
    }
}

mod controller;
mod grouping;
