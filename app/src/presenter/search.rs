use std::collections::HashMap;

use icu_normalizer::ComposingNormalizerBorrowed;

/// All lists search the same normalized names/titles, including romaji readings.
pub(super) struct Query {
    text: String,
    kana: Vec<String>,
}

/// Keep folds across keystrokes; discard names no list used during this present.
#[derive(Default)]
pub(super) struct Fields(HashMap<String, Field>);

struct Field {
    text: String,
    kana: String,
    used: bool,
}

impl Fields {
    pub(super) fn finish(&mut self) {
        self.0.retain(|_, field| std::mem::take(&mut field.used));
    }

    fn get(&mut self, name: &str) -> &Field {
        let field = self.0.entry(name.to_owned()).or_insert_with(|| fold(name));
        field.used = true;
        field
    }
}

impl Query {
    pub(super) fn new(text: &str) -> Self {
        let Field { text, kana, .. } = fold(text);
        let mut forms = vec![kana];
        // An unfinished syllable is a prefix: `tan` can mean `ta` + `na`, not just `tan`.
        let trailing = text.trim_end_matches(|c: char| is_consonant(c));
        let suffix = &text[trailing.len()..];
        if !suffix.is_empty() && suffix.len() <= 2 {
            for vowel in [
                "a", "i", "u", "e", "o", "ya", "yu", "yo", "ha", "hi", "hu", "ho",
            ] {
                let syllable = format!("{suffix}{vowel}");
                if let Some(kana) = syllable_kana(&syllable) {
                    forms.push(fold(&format!("{trailing}{kana}")).kana);
                }
            }
        }
        Self { text, kana: forms }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(super) fn matches<'a>(
        &self,
        fields: impl IntoIterator<Item = &'a str>,
        cache: &mut Fields,
    ) -> bool {
        fields.into_iter().any(|field| {
            let Field { text, kana, .. } = cache.get(field);
            text.contains(&self.text) || self.kana.iter().any(|query| kana.contains(query.as_str()))
        })
    }
}

fn normalize(text: &str) -> String {
    ComposingNormalizerBorrowed::new_nfkc()
        .normalize(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|c| match c {
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap(),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace('ß', "ss")
        .replace('ς', "σ")
}

fn fold(text: &str) -> Field {
    let text = normalize(text);
    let mut kana = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap();
        let next = rest.as_bytes().get(1).copied();
        if c.is_ascii()
            && is_consonant(c)
            && c != 'n'
            && (next == Some(c as u8) || rest.starts_with("tch"))
        {
            kana.push('っ');
            rest = &rest[1..];
        } else if c == 'n'
            && (rest.len() == 1
                || rest.starts_with("n'")
                || next.is_some_and(|n| !b"aiueoy".contains(&n)))
        {
            kana.push('ん');
            let skip = if rest.starts_with("n'")
                || (rest.starts_with("nn")
                    && !rest
                        .as_bytes()
                        .get(2)
                        .is_some_and(|n| b"aiueoy".contains(n)))
            {
                2
            } else {
                1
            };
            rest = &rest[skip..];
        } else {
            let syllable = [3, 2, 1]
                .into_iter()
                .find_map(|len| rest.get(..len).and_then(syllable_kana).map(|k| (len, k)));
            if let Some((len, k)) = syllable {
                kana.push_str(k);
                rest = &rest[len..];
            } else {
                kana.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    let mut folded = String::with_capacity(kana.len());
    let mut vowel = None;
    for c in kana.chars() {
        let c = match c {
            'ぁ' => 'あ',
            'ぃ' => 'い',
            'ぅ' => 'う',
            'ぇ' => 'え',
            'ぉ' => 'お',
            'ゃ' => 'や',
            'ゅ' => 'ゆ',
            'ょ' => 'よ',
            'っ' => 'つ',
            'ゎ' => 'わ',
            'ー' => vowel.unwrap_or('ー'),
            _ => c,
        };
        let c = if c == 'う' && vowel == Some('お') {
            'お'
        } else {
            c
        };
        folded.push(c);
        vowel = kana_vowel(c);
    }
    Field {
        text,
        kana: folded,
        used: true,
    }
}

fn kana_vowel(c: char) -> Option<char> {
    [
        ("あかがさざただなはばぱまやらわゕ", 'あ'),
        ("いきぎしじちぢにひびぴみりゐ", 'い'),
        ("うくぐすずつづぬふぶぷむゆるゔ", 'う'),
        ("えけげせぜてでねへべぺめれゑゖ", 'え'),
        ("おこごそぞとどのほぼぽもよろを", 'お'),
    ]
    .into_iter()
    .find_map(|(kana, vowel)| kana.contains(c).then_some(vowel))
}

fn is_consonant(c: char) -> bool {
    c.is_ascii_lowercase() && !"aiueo".contains(c)
}

fn syllable_kana(s: &str) -> Option<&'static str> {
    Some(match s {
        "a" => "あ",
        "i" => "い",
        "u" => "う",
        "e" => "え",
        "o" => "お",
        "ka" => "か",
        "ki" => "き",
        "ku" => "く",
        "ke" => "け",
        "ko" => "こ",
        "ga" => "が",
        "gi" => "ぎ",
        "gu" => "ぐ",
        "ge" => "げ",
        "go" => "ご",
        "sa" => "さ",
        "shi" | "si" => "し",
        "su" => "す",
        "se" => "せ",
        "so" => "そ",
        "za" => "ざ",
        "ji" | "zi" => "じ",
        "zu" => "ず",
        "ze" => "ぜ",
        "zo" => "ぞ",
        "ta" => "た",
        "chi" | "ti" => "ち",
        "tsu" | "tu" => "つ",
        "te" => "て",
        "to" => "と",
        "da" => "だ",
        "di" => "ぢ",
        "du" => "づ",
        "de" => "で",
        "do" => "ど",
        "na" => "な",
        "ni" => "に",
        "nu" => "ぬ",
        "ne" => "ね",
        "no" => "の",
        "ha" => "は",
        "hi" => "ひ",
        "fu" | "hu" => "ふ",
        "he" => "へ",
        "ho" => "ほ",
        "ba" => "ば",
        "bi" => "び",
        "bu" => "ぶ",
        "be" => "べ",
        "bo" => "ぼ",
        "pa" => "ぱ",
        "pi" => "ぴ",
        "pu" => "ぷ",
        "pe" => "ぺ",
        "po" => "ぽ",
        "ma" => "ま",
        "mi" => "み",
        "mu" => "む",
        "me" => "め",
        "mo" => "も",
        "ya" => "や",
        "yu" => "ゆ",
        "yo" => "よ",
        "ra" => "ら",
        "ri" => "り",
        "ru" => "る",
        "re" => "れ",
        "ro" => "ろ",
        "wa" => "わ",
        "wi" => "うぃ",
        "we" => "うぇ",
        "wo" => "を",
        "kya" => "きゃ",
        "kyu" => "きゅ",
        "kyo" => "きょ",
        "gya" => "ぎゃ",
        "gyu" => "ぎゅ",
        "gyo" => "ぎょ",
        "sha" | "sya" => "しゃ",
        "shu" | "syu" => "しゅ",
        "sho" | "syo" => "しょ",
        "ja" | "jya" | "zya" => "じゃ",
        "ju" | "jyu" | "zyu" => "じゅ",
        "jo" | "jyo" | "zyo" => "じょ",
        "cha" | "tya" | "cya" => "ちゃ",
        "chu" | "tyu" | "cyu" => "ちゅ",
        "cho" | "tyo" | "cyo" => "ちょ",
        "dya" => "ぢゃ",
        "dyu" => "ぢゅ",
        "dyo" => "ぢょ",
        "nya" => "にゃ",
        "nyu" => "にゅ",
        "nyo" => "にょ",
        "hya" => "ひゃ",
        "hyu" => "ひゅ",
        "hyo" => "ひょ",
        "bya" => "びゃ",
        "byu" => "びゅ",
        "byo" => "びょ",
        "pya" => "ぴゃ",
        "pyu" => "ぴゅ",
        "pyo" => "ぴょ",
        "mya" => "みゃ",
        "myu" => "みゅ",
        "myo" => "みょ",
        "rya" => "りゃ",
        "ryu" => "りゅ",
        "ryo" => "りょ",
        "va" => "ゔぁ",
        "vi" => "ゔぃ",
        "vu" => "ゔ",
        "ve" => "ゔぇ",
        "vo" => "ゔぉ",
        "fa" => "ふぁ",
        "fi" => "ふぃ",
        "fe" => "ふぇ",
        "fo" => "ふぉ",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual UI-thread filtering benchmark; prints timings without a timing assertion"]
    fn benchmark_thousand_chat_rows() {
        use std::{hint::black_box, time::Instant};
        let rows: Vec<_> = (0..1000)
            .map(|i| {
                format!(
                    "{} {i}",
                    [
                        "田中 タナカ",
                        "ﾗｰﾒﾝ同好会",
                        "Tanaka study group",
                        "東京 とうきょう",
                        "キャラクターのお部屋"
                    ][i % 5]
                )
            })
            .collect();
        let mut cache = Fields::default();
        for row in &rows {
            cache.get(row);
        }
        cache.finish();
        for text in ["tan", "らーめん", "tookyoo", "きゃ", "absent"] {
            let mut timings = Vec::new();
            for _ in 0..30 {
                let start = Instant::now();
                let query = Query::new(text);
                black_box(
                    rows.iter()
                        .filter(|row| query.matches([row.as_str()], &mut cache))
                        .count(),
                );
                cache.finish();
                timings.push(start.elapsed());
            }
            timings.sort();
            eprintln!(
                "{text:?}: 1000 rows median {:?}, max {:?}",
                timings[15], timings[29]
            );
        }
    }

    #[test]
    fn query_folds_case_and_whitespace_on_both_sides() {
        for (query, fields, expected) in [
            ("  ALIce \t SMith\n", ["Other", "Alice\n  Smith"], true),
            ("smith", ["Alice Smith", "Other"], true),
            (" \t ", ["Alice Smith", "Other"], true),
            ("Bob", ["Alice Smith", "Other"], false),
        ] {
            assert_eq!(
                Query::new(query).matches(fields, &mut Fields::default()),
                expected,
                "{query:?}"
            );
        }
    }

    #[test]
    fn japanese_queries_match_normalized_names() {
        for (query, field, expected) in [
            ("abc", "ＡＢＣ", true),
            ("ＡＢＣ", "abc", true),
            ("1", "①", true),
            ("ケーキ", "けえき", true),
            ("しい", "シー", true),
            ("すう", "スー", true),
            ("おお", "オー", true),
            ("らーめん", "ラーメン", true),
            ("らーめん", "ﾗｰﾒﾝ", true),
            ("らーめん", "らあめん", true),
            ("らあめん", "ラーメン", true),
            ("きゃ", "キャ", true),
            ("きや", "ｷｬ", true),
            ("つあいうえおやゆよわ", "ッァィゥェォャュョヮ", true),
            ("ゔ", "ヴ", true),
            ("が", "ｶﾞ", true),
            ("が", "か", false),
            ("か", "ガ", false),
            ("は", "パ", false),
            ("ば", "ぱ", false),
            ("shi", "シ", true),
            ("si", "し", true),
            ("chi", "チ", true),
            ("ti", "ち", true),
            ("tsu", "ツ", true),
            ("tu", "つ", true),
            ("fu", "フ", true),
            ("hu", "ふ", true),
            ("ji", "ジ", true),
            ("zi", "じ", true),
            ("きゃ", "kya", true),
            ("sya", "シャ", true),
            ("tyu", "チュ", true),
            ("zya", "ジャ", true),
            ("matcha", "まっちゃ", true),
            ("sh", "しお", true),
            ("ky", "きゃく", true),
            ("sm", "Smith", true),
            ("STRASSE", "Straße", true),
            ("gakkou", "ガッコー", true),
            ("がっこう", "gakkoo", true),
            ("しんぶん", "shinbun", true),
            ("しんぶん", "shinnbunn", true),
            ("kanna", "かんな", true),
            ("kan'i", "かんい", true),
            ("toukyou", "トーキョー", true),
            ("tookyoo", "とうきょう", true),
            ("くう", "クー", true),
            ("kuukou", "クーコー", true),
            ("tan", "たなか", true),
            ("t", "たなか", true),
            ("c", "ちか", true),
            ("tan", "たん", true),
            ("tann", "たなか", false),
            ("tan", "たかはし", false),
            ("tanaか", "タナカ", true),
            ("たなか", "Tanaka", true),
            ("が", "ka", false),
            ("か", "ga", false),
            ("やまだ", "山田", false),
        ] {
            assert_eq!(
                Query::new(query).matches([field], &mut Fields::default()),
                expected,
                "{query:?} / {field:?}"
            );
        }
    }
}
