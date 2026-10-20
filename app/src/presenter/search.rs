/// Every list searches names/titles with the same whitespace and case rules.
pub(super) struct Query(String);

impl Query {
    pub(super) fn new(text: &str) -> Self {
        Self(fold(text))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn matches<'a>(&self, fields: impl IntoIterator<Item = &'a str>) -> bool {
        fields
            .into_iter()
            .any(|field| fold(field).contains(&self.0))
    }
}

fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_folds_case_and_whitespace_on_both_sides() {
        for (query, fields, expected) in [
            ("  ALIce \t SMith\n", ["Other", "Alice\n  Smith"], true),
            ("smith", ["Alice Smith", "Other"], true),
            (" \t ", ["Alice Smith", "Other"], true),
            ("Bob", ["Alice Smith", "Other"], false),
        ] {
            assert_eq!(Query::new(query).matches(fields), expected, "{query:?}");
        }
    }
}
