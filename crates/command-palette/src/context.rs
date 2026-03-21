use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Gmail,
    Jmap,
    Graph,
    Imap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FocusedRegion {
    ThreadList,
    ReadingPane,
    Composer,
    SearchBar,
    Sidebar,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewType {
    #[default]
    Inbox,
    Starred,
    Sent,
    Drafts,
    Snoozed,
    Trash,
    Spam,
    AllMail,
    Label,
    SmartFolder,
    Category,
    Tasks,
    Calendar,
    Settings,
    Attachments,
    Search,
    PinnedSearch,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandContext {
    pub selected_thread_ids: Vec<String>,
    pub active_message_id: Option<String>,

    pub current_view: ViewType,
    pub current_label_id: Option<String>,

    pub active_account_id: Option<String>,
    pub provider_kind: Option<ProviderKind>,

    pub thread_is_read: Option<bool>,
    pub thread_is_starred: Option<bool>,
    pub thread_is_muted: Option<bool>,
    pub thread_is_pinned: Option<bool>,
    pub thread_is_draft: Option<bool>,
    pub thread_in_trash: Option<bool>,
    pub thread_in_spam: Option<bool>,

    pub is_online: bool,
    pub composer_is_open: bool,

    pub focused_region: Option<FocusedRegion>,
}

impl CommandContext {
    pub fn has_selection(&self) -> bool {
        !self.selected_thread_ids.is_empty()
    }

    pub fn has_single_selection(&self) -> bool {
        self.selected_thread_ids.len() == 1
    }

    pub fn selection_count(&self) -> usize {
        self.selected_thread_ids.len()
    }

    pub fn is_focused(&self, region: FocusedRegion) -> bool {
        self.focused_region == Some(region)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_context() -> CommandContext {
        CommandContext {
            selected_thread_ids: vec![],
            active_message_id: None,
            current_view: ViewType::Inbox,
            current_label_id: None,
            active_account_id: None,
            provider_kind: None,
            thread_is_read: None,
            thread_is_starred: None,
            thread_is_muted: None,
            thread_is_pinned: None,
            thread_is_draft: None,
            thread_in_trash: None,
            thread_in_spam: None,
            is_online: true,
            composer_is_open: false,
            focused_region: None,
        }
    }

    #[test]
    fn no_selection() {
        let ctx = empty_context();
        assert!(!ctx.has_selection());
        assert!(!ctx.has_single_selection());
        assert_eq!(ctx.selection_count(), 0);
    }

    #[test]
    fn single_selection() {
        let mut ctx = empty_context();
        ctx.selected_thread_ids = vec!["t1".to_string()];
        assert!(ctx.has_selection());
        assert!(ctx.has_single_selection());
        assert_eq!(ctx.selection_count(), 1);
    }

    #[test]
    fn multi_selection() {
        let mut ctx = empty_context();
        ctx.selected_thread_ids = vec!["t1".to_string(), "t2".to_string()];
        assert!(ctx.has_selection());
        assert!(!ctx.has_single_selection());
        assert_eq!(ctx.selection_count(), 2);
    }
}
