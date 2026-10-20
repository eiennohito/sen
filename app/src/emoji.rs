fn path(fonts: &std::path::Path) -> std::path::PathBuf {
    #[cfg(any(debug_assertions, feature = "ui-debug"))]
    if let Some(path) = std::env::var_os("SEN_EMOJI_FONT") {
        return path.into();
    }
    fonts.join("NotoColorEmoji.ttf")
}

pub(crate) fn load(fonts: &std::path::Path) {
    match std::fs::read(path(fonts)) {
        Ok(data) => register(data),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => sen::reporting::log("emoji-font", "system-fallback", &error),
    }
}

fn register(data: Vec<u8>) {
    use slint::fontique_011::{
        fontique::{Blob, GenericFamily},
        shared_collection,
    };
    let mut collection = shared_collection();
    let registered = collection.register_fonts(Blob::new(std::sync::Arc::new(data)), None);
    if registered.is_empty() {
        sen::reporting::log(
            "emoji-font",
            "system-fallback",
            &std::io::Error::new(std::io::ErrorKind::InvalidData, "no fonts registered"),
        );
        return;
    }
    let old: Vec<_> = collection.generic_families(GenericFamily::Emoji).collect();
    collection.set_generic_families(
        GenericFamily::Emoji,
        registered.iter().map(|(id, _)| *id).chain(old),
    );
}

#[cfg(test)]
mod tests {
    use slint::ComponentHandle;

    slint::slint! {
        export component EmojiSample inherits Window {
            in property <string> sample;
            in property <bool> stroked;
            in property <float> fade: 1;
            background: white;
            Text {
                text: root.sample;
                opacity: root.fade;
                stroke: root.stroked ? red : transparent;
                stroke-width: 1px;
                color: #303030;
                font-family: "DejaVu Sans";
                font-size: 40px;
                horizontal-alignment: center;
                vertical-alignment: center;
            }
        }
    }

    #[test]
    fn missing_private_font_is_quiet_but_unreadable_and_invalid_fonts_are_logged() {
        const CHILD: &str = "SEN_TEST_EMOJI_FALLBACK";
        if std::env::var_os(CHILD).is_some() {
            let directory = sen::test_support::TempDir::new("emoji-fallback");
            super::load(&directory.0);
            let font = directory.0.join("NotoColorEmoji.ttf");
            std::fs::create_dir(&font).unwrap();
            super::load(&directory.0);
            std::fs::remove_dir(&font).unwrap();
            std::fs::write(&font, b"invalid font").unwrap();
            super::load(&directory.0);
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "emoji::tests::missing_private_font_is_quiet_but_unreadable_and_invalid_fonts_are_logged", "--nocapture"])
            .env(CHILD, "1")
            .env_remove("SEN_EMOJI_FONT")
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stderr}");
        assert_eq!(
            stderr
                .matches("operation=emoji-font kind=system-fallback")
                .count(),
            2,
            "{stderr}"
        );
        assert!(!stderr.contains("io:NotFound"), "{stderr}");
        assert!(stderr.contains("io:InvalidData"), "{stderr}");
    }

    #[test]
    fn emoji_presentation_renders_in_colour_and_text_presentation_stays_grey() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fonts/NotoColorEmoji.ttf");
        assert!(
            path.is_file(),
            "missing {}; run python3 tools/fetch_emoji_font.py",
            path.display()
        );
        let data = std::fs::read(path).unwrap();
        std::thread::spawn(move || {
            let window = crate::snapshot::install().unwrap();
            super::register(data);
            let ui = EmojiSample::new().unwrap();
            ui.show().unwrap();
            for (sample, colour) in [
                ("🌸", true),
                ("😀", true),
                ("❤️", true),
                ("❤︎", false),
                ("👩‍💻", true),
                ("🇯🇵", true),
                ("1️⃣", true),
            ] {
                ui.set_sample(sample.into());
                let pixels = crate::snapshot::render(&window, (100, 80)).unwrap();
                let coloured = pixels
                    .as_slice()
                    .iter()
                    .filter(|p| p.r.max(p.g).max(p.b) - p.r.min(p.g).min(p.b) > 20)
                    .count();
                let ink = pixels
                    .as_slice()
                    .iter()
                    .filter(|p| p.r.min(p.g).min(p.b) < 150)
                    .count();
                assert!(ink > 20, "{sample:?} is blank");
                if colour {
                    assert!(
                        coloured > 20,
                        "{sample:?} lost its colour ({coloured} pixels)"
                    );
                } else {
                    assert_eq!(coloured, 0, "{sample:?} lost text presentation");
                }
            }
            ui.set_sample("🌸".into());
            let opaque = crate::snapshot::render(&window, (100, 80)).unwrap();
            ui.set_fade(0.5);
            let faded = crate::snapshot::render(&window, (100, 80)).unwrap();
            for (full, half) in opaque.as_bytes().iter().zip(faded.as_bytes()) {
                let expected = (u16::from(*full) + 255) / 2;
                assert!(
                    (i32::from(*half) - i32::from(expected)).abs() <= 2,
                    "colour glyph ignored global opacity"
                );
            }
            ui.set_stroked(true);
            let stroked = crate::snapshot::render(&window, (100, 80)).unwrap();
            assert_eq!(
                faded.as_bytes(),
                stroked.as_bytes(),
                "stroke drew the colour glyph twice"
            );
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }
}
