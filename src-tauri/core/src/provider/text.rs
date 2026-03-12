use unicode_segmentation::UnicodeSegmentation;

pub fn truncate_graphemes(text: &str, max_graphemes: usize) -> String {
    UnicodeSegmentation::graphemes(text, true)
        .take(max_graphemes)
        .collect()
}

pub fn snippet_from_text_body(text: &str, max_graphemes: usize) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let trimmed = collapsed.trim();
    let truncated = truncate_graphemes(trimmed, max_graphemes);
    if truncated.len() < trimmed.len() {
        format!("{truncated}...")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{snippet_from_text_body, truncate_graphemes};

    #[test]
    fn truncates_without_splitting_grapheme_clusters() {
        let text = "Hi 👨‍👩‍👧‍👦 there";
        assert_eq!(truncate_graphemes(text, 4), "Hi 👨‍👩‍👧‍👦");
    }

    #[test]
    fn snippet_collapses_whitespace_and_appends_ellipsis() {
        let text = "hello\n\nworld";
        assert_eq!(snippet_from_text_body(text, 5), "hello...");
    }
}
