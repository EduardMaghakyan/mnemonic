use unicode_normalization::UnicodeNormalization;

pub fn title_to_slug(title: &str, fallback_timestamp: &str) -> String {
    let folded: String = title.nfkd().filter(|c| c.is_ascii()).collect();
    let lowered = folded.to_lowercase();

    let mut compacted = String::with_capacity(lowered.len());
    let mut last_dash = false;
    for c in lowered.chars() {
        if c.is_ascii_alphanumeric() {
            compacted.push(c);
            last_dash = false;
        } else if !last_dash {
            compacted.push('-');
            last_dash = true;
        }
    }
    let trimmed = compacted.trim_matches('-');

    let truncated = if trimmed.len() <= 60 {
        trimmed.to_string()
    } else {
        match trimmed[..60].rfind('-') {
            Some(idx) if idx > 0 => trimmed[..idx].to_string(),
            _ => trimmed[..60].to_string(),
        }
    };

    if truncated.is_empty() {
        format!("note-{fallback_timestamp}")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_lowercase_with_dashes() {
        assert_eq!(title_to_slug("Hello World", "X"), "hello-world");
    }

    #[test]
    fn folds_diacritics_via_nfkd() {
        assert_eq!(title_to_slug("Café résumé", "X"), "cafe-resume");
    }

    #[test]
    fn collapses_runs_of_non_alphanumeric() {
        assert_eq!(title_to_slug("foo!!  bar...baz", "X"), "foo-bar-baz");
    }

    #[test]
    fn trims_leading_and_trailing_dashes() {
        assert_eq!(title_to_slug("--- hello ---", "X"), "hello");
    }

    #[test]
    fn truncates_to_60_at_word_boundary() {
        let title = "one two three four five six seven eight nine ten eleven twelve";
        let slug = title_to_slug(title, "X");
        assert!(slug.len() <= 60, "len was {}: {slug}", slug.len());
        assert!(!slug.ends_with('-'));
        assert!(slug.starts_with("one-two"));
    }

    #[test]
    fn truncates_hard_when_no_dash_in_first_60() {
        let title = "a".repeat(70);
        let slug = title_to_slug(&title, "X");
        assert_eq!(slug.len(), 60);
    }

    #[test]
    fn falls_back_when_empty_after_normalization() {
        assert_eq!(title_to_slug("中文", "20260508-013000"), "note-20260508-013000");
        assert_eq!(title_to_slug("---", "20260508-013000"), "note-20260508-013000");
    }

    #[test]
    fn keeps_digits() {
        assert_eq!(title_to_slug("Q3 2026 plan", "X"), "q3-2026-plan");
    }
}
