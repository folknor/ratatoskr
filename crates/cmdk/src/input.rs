use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

/// Value/label pair for static enum choices in `ParamDef::Enum`.
///
/// Separates machine identifier (`value`) from display text (`label`).
/// The frontend sends `value` back, not `label`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumOption {
    pub value: &'static str,
    pub label: &'static str,
}

/// Describes one parameter step in a parameterized command's input flow.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamDef {
    /// Pick one item from a dynamic list (folders, labels, accounts).
    /// Options fetched via `CommandInputResolver::get_options()`.
    ListPicker { label: &'static str },
    /// Pick a date/time (snooze). Frontend renders a date picker.
    DateTime { label: &'static str },
    /// Pick from a fixed set of options defined in the schema.
    Enum {
        label: &'static str,
        options: &'static [EnumOption],
    },
    /// Free text input (rename folder, search query).
    Text {
        label: &'static str,
        placeholder: &'static str,
    },
}

/// What parameters a command requires before execution.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputSchema {
    /// One parameter, then done.
    Single { param: ParamDef },
    /// Multiple parameters resolved sequentially.
    Sequence { params: &'static [ParamDef] },
}

impl InputSchema {
    /// Return the `ParamDef` at the given index, or `None` if out of bounds.
    pub fn param_at(&self, index: usize) -> Option<ParamDef> {
        match self {
            Self::Single { param } if index == 0 => Some(*param),
            Self::Sequence { params } => params.get(index).copied(),
            _ => None,
        }
    }

    /// Total number of parameter steps.
    pub fn len(&self) -> usize {
        match self {
            Self::Single { .. } => 1,
            Self::Sequence { params } => params.len(),
        }
    }

    /// Whether this schema has zero steps (always false for valid schemas).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ParamDef {
    /// Whether this step requires dynamic option resolution via the resolver.
    pub fn is_list_picker(&self) -> bool {
        matches!(self, Self::ListPicker { .. })
    }
}

/// Carried on `CommandMatch`, tells the frontend what to do after the user
/// picks a command from the palette.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputMode {
    /// No parameters needed — execute immediately with `CommandId` alone.
    Direct,
    /// Requires parameter resolution before execution.
    Parameterized { schema: InputSchema },
}

/// A single option returned by the resolver for a `ListPicker` step.
///
/// This is runtime data fetched from the DB — it allocates. The registry's
/// static types (`ParamDef`, `InputSchema`, `InputMode`) are `Copy` and
/// zero-allocation. Different layers, different allocation profiles.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionItem {
    /// Stable identifier for execution (folder ID, label ID).
    pub id: String,
    /// Leaf display name ("Reviews").
    pub label: String,
    /// Breadcrumb path for hierarchical display (["Projects", "Q2", "Reviews"]).
    /// Included in search text so ancestor names are searchable.
    pub path: Option<Vec<String>>,
    /// Additional search terms (aliases, alternative names).
    pub keywords: Option<Vec<String>>,
    /// Greyed out but visible (e.g., can't move to current folder).
    pub disabled: bool,
}

/// An `OptionItem` with fuzzy match scoring, returned by `search_options`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionMatch {
    pub item: OptionItem,
    pub score: u32,
    /// The concatenated search haystack that was matched against.
    /// Format: `"{path} > {label} {keywords}"` (path/keywords omitted if absent).
    /// `match_positions` are byte offsets into this string.
    pub match_text: String,
    pub match_positions: Vec<u32>,
}

/// Fuzzy search over a list of `OptionItem`s using nucleo-matcher.
///
/// Search haystack per item: `"{path} > {label} {keywords}"`.
/// - `path` segments joined with " > " (omitted if None)
/// - `keywords` space-separated (omitted if None)
///
/// When `query` is empty: returns all items with score 0, preserving order.
/// When non-empty: returns only matches, sorted by score descending.
pub fn search_options(items: &[OptionItem], query: &str) -> Vec<OptionMatch> {
    if query.is_empty() {
        return items
            .iter()
            .map(|item| OptionMatch {
                match_text: build_haystack(item),
                item: item.clone(),
                score: 0,
                match_positions: vec![],
            })
            .collect();
    }
    search_options_fuzzy(items, query)
}

fn search_options_fuzzy(items: &[OptionItem], query: &str) -> Vec<OptionMatch> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut indices = Vec::new();
    let mut results = Vec::new();

    for item in items {
        let haystack_str = build_haystack(item);
        let haystack = Utf32Str::new(&haystack_str, &mut buf);

        if let Some(score) = pattern.score(haystack, &mut matcher) {
            indices.clear();
            indices.resize(pattern.atoms.len() * 2, 0);
            pattern.indices(haystack, &mut matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();

            results.push(OptionMatch {
                item: item.clone(),
                score,
                match_text: haystack_str,
                match_positions: indices.clone(),
            });
        }
    }

    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    results
}

fn build_haystack(item: &OptionItem) -> String {
    let mut haystack = String::new();

    if let Some(path) = &item.path {
        for (i, segment) in path.iter().enumerate() {
            if i > 0 {
                haystack.push_str(" > ");
            }
            haystack.push_str(segment);
        }
        haystack.push_str(" > ");
    }

    haystack.push_str(&item.label);

    if let Some(keywords) = &item.keywords {
        for kw in keywords {
            haystack.push(' ');
            haystack.push_str(kw);
        }
    }

    haystack
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: &str, label: &str) -> OptionItem {
        OptionItem {
            id: id.to_string(),
            label: label.to_string(),
            path: None,
            keywords: None,
            disabled: false,
        }
    }

    fn make_item_with_path(id: &str, label: &str, path: Vec<&str>) -> OptionItem {
        OptionItem {
            id: id.to_string(),
            label: label.to_string(),
            path: Some(path.into_iter().map(String::from).collect()),
            keywords: None,
            disabled: false,
        }
    }

    fn make_item_with_keywords(id: &str, label: &str, keywords: Vec<&str>) -> OptionItem {
        OptionItem {
            id: id.to_string(),
            label: label.to_string(),
            path: None,
            keywords: Some(keywords.into_iter().map(String::from).collect()),
            disabled: false,
        }
    }

    #[test]
    fn search_options_empty_query_returns_all() {
        let items = vec![make_item("1", "Inbox"), make_item("2", "Sent")];
        let results = search_options(&items, "");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].score, 0);
        assert_eq!(results[1].score, 0);
        assert_eq!(results[0].item.id, "1");
        assert_eq!(results[1].item.id, "2");
    }

    #[test]
    fn search_options_filters_by_label() {
        let items = vec![
            make_item("1", "Inbox"),
            make_item("2", "Sent"),
            make_item("3", "Trash"),
        ];
        let results = search_options(&items, "Inbox");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.id, "1");
        assert!(results[0].score > 0);
    }

    #[test]
    fn search_options_matches_keywords() {
        let items = vec![
            make_item("1", "Main"),
            make_item_with_keywords("2", "Primary", vec!["main", "default"]),
        ];
        let results = search_options(&items, "default");
        assert!(results.iter().any(|r| r.item.id == "2"));
    }

    #[test]
    fn search_options_matches_path() {
        let items = vec![
            make_item("1", "Inbox"),
            make_item_with_path("2", "Reviews", vec!["Projects", "Q2"]),
        ];
        let results = search_options(&items, "q2");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.id, "2");
        assert!(results[0].score > 0);
    }

    #[test]
    fn search_options_excludes_non_matching() {
        let items = vec![
            make_item("1", "Inbox"),
            make_item("2", "Sent"),
        ];
        let results = search_options(&items, "zzzzz");
        assert!(results.is_empty());
    }

    #[test]
    fn search_options_sorted_by_score_desc() {
        let items = vec![
            make_item("1", "Archive"),
            make_item("2", "Archived Items"),
            make_item("3", "Archive All"),
        ];
        let results = search_options(&items, "archive");
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "results not sorted by score desc"
            );
        }
    }

    #[test]
    fn search_options_handles_empty_list() {
        let results = search_options(&[], "anything");
        assert!(results.is_empty());
    }
}
