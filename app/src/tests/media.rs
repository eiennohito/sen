use super::*;
use line_wasm_poc::client::desktop::assets::{Completion, Failure, Purpose};

#[test]
fn media_completion_classes_reach_placeholders_and_retry_actions() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View {
            current: "chat".into(),
            demo: true,
            ..Default::default()
        };
        let cases = [
            (Failure::Timeout, "Image took too long to load", true),
            (Failure::Network, "Couldn’t load this image", true),
            (Failure::Http(404), "Not available for this sender", false),
            (
                Failure::Http(403),
                "You don’t have access to this image",
                true,
            ),
            (Failure::Http(500), "Image is temporarily unavailable", true),
            (
                Failure::UnsupportedHost,
                "Not available for this sender",
                false,
            ),
            (Failure::RejectedPath, "Image is unavailable", false),
            (Failure::TooLarge, "Image too large", false),
            (Failure::Decode, "Couldn’t display this image", false),
            (Failure::Cancelled, "Image loading cancelled", true),
        ];
        for (failure, text, retryable) in cases {
            view.messages = vec![MessageRow {
                id: "1".into(),
                image_key: "image".into(),
                photo: true,
                ..Default::default()
            }];
            view.recent.insert("chat".into(), view.messages.clone());
            let events = RefCell::new(Vec::new());
            account::publish_asset(
                Completion {
                    key: "image".into(),
                    purpose: Purpose::Thumbnail,
                    result: Err(failure),
                },
                &|event| events.borrow_mut().push(event),
            );
            view.update(&ui, events.borrow_mut().pop().unwrap());
            let row = ui.get_messages().row_data(0).unwrap();
            assert_eq!(row.image_error, text);
            assert_eq!(row.image_retryable, retryable);
            assert_eq!(row.image_failure, format!("{failure:?}"));
            assert_eq!(view.recent["chat"][0].image_error, text);
            view.retry_image(&ui, "image");
            assert_eq!(view.messages[0].has_picture, retryable);
            assert_eq!(view.recent["chat"][0].has_picture, retryable);
            assert_eq!(view.messages[0].image_failed, !retryable);
            view.viewer_request = 2;
            ui.set_viewer_open(true);
            account::publish_asset(
                Completion {
                    key: "original".into(),
                    purpose: Purpose::Original { request: 2 },
                    result: Err(failure),
                },
                &|event| events.borrow_mut().push(event),
            );
            view.update(&ui, events.borrow_mut().pop().unwrap());
            assert_eq!(ui.get_viewer_error(), text);
            assert_eq!(ui.global::<State>().get_viewer_retryable(), retryable);
            ui.set_viewer_open(false);
            view.file_request = 3;
            ui.global::<State>().set_file_save_id("1".into());
            account::publish_asset(
                Completion {
                    key: "file".into(),
                    purpose: Purpose::File {
                        request: 3,
                        name: "document".into(),
                    },
                    result: Err(failure),
                },
                &|event| events.borrow_mut().push(event),
            );
            view.update(&ui, events.borrow_mut().pop().unwrap());
            assert_eq!(view.messages[0].file_error, failure.file_text());
            assert_eq!(view.messages[0].file_retryable, retryable);
            assert_eq!(view.messages[0].file_failure, format!("{failure:?}"));
            assert_eq!(view.recent["chat"][0].file_error, failure.file_text());
        }
    })
    .join()
    .unwrap();
}
