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
        decryption_failed: false,
        mentioned: false,
        skip_badge: false,
        emoji: Vec::new(),
        outgoing: false,
        image_key: String::new(),
        reactions: Vec::new(),
    }
}

mod connection;
mod controller;
mod grouping;

mod composer;

mod layers;

mod paging;

mod media;
