// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::{FontContext, FontFamily, LayoutContext};
use std::sync::Arc;

#[test]
fn emoji_presentation_prefers_emoji_family_over_text_coverage() {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let text_blob = Blob::new(Arc::new(include_bytes!("fonts/text.ttf").to_vec()));
    let emoji_blob = Blob::new(Arc::new(include_bytes!("fonts/emoji.ttf").to_vec()));
    let text_id = text_blob.id();
    let emoji_id = emoji_blob.id();
    collection.register_fonts(text_blob, None);
    let families = collection.register_fonts(emoji_blob, None);
    collection.set_generic_families(GenericFamily::Emoji, families.iter().map(|(id, _)| *id));
    let mut fonts = FontContext {
        collection,
        source_cache: SourceCache::default(),
    };
    let mut context = LayoutContext::<[u8; 4]>::new();
    for (sample, expected) in [
        ("🌸", emoji_id),
        ("😀", emoji_id),
        ("☕", emoji_id),
        ("❤️", emoji_id),
        ("🐈", emoji_id),
        ("👍", emoji_id),
        ("👩‍💻", emoji_id),
        ("🇯🇵", emoji_id),
        ("1️⃣", emoji_id),
        ("1", text_id),
        ("#", text_id),
        ("©", text_id),
        ("1\u{20e3}", emoji_id),
        ("❤", text_id),
        ("☺", text_id),
        ("❤︎", text_id),
        ("😀︎", text_id),
    ] {
        let mut builder = context.ranged_builder(&mut fonts, sample, 1.0, false);
        builder.push_default(FontFamily::from("Test Text"));
        let mut layout = builder.build(sample);
        layout.break_all_lines(None);
        assert!(
            layout.lines().flat_map(|line| line.runs()).next().is_some(),
            "no runs for {sample:?}"
        );
        for run in layout.lines().flat_map(|line| line.runs()) {
            assert_eq!(
                run.font().data.id(),
                expected,
                "wrong presentation for {sample:?}"
            );
        }
    }
    let sample = "a😀b";
    let mut builder = context.ranged_builder(&mut fonts, sample, 1.0, false);
    builder.push_default(FontFamily::from("Test Text"));
    let mut layout = builder.build(sample);
    layout.break_all_lines(None);
    let runs: Vec<_> = layout
        .lines()
        .flat_map(|line| line.runs())
        .map(|run| (run.text_range(), run.font().data.id()))
        .collect();
    assert_eq!(runs, [(0..1, text_id), (1..5, emoji_id), (5..6, text_id)]);
}
