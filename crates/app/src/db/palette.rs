use super::connection::Db;

// ── Palette query methods ────────────────────────────────────

impl Db {
    /// User-visible folders/labels for an account, excluding system labels.
    ///
    /// For Gmail, splits `/`-delimited labels into path segments.
    /// Returns `OptionItem`s for the palette's ListPicker stage 2.
    pub fn get_user_folders_for_palette(
        &self,
        account_id: &str,
    ) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_conn_sync(|conn| rtsk::command_palette_queries::get_user_folders_for_palette(conn, account_id))
    }

    /// All user labels for an account (same as folders for now).
    pub fn get_user_labels_for_palette(
        &self,
        account_id: &str,
    ) -> Result<Vec<cmdk::OptionItem>, String> {
        self.get_user_folders_for_palette(account_id)
    }

    /// Labels currently applied to a specific thread.
    pub fn get_thread_labels_for_palette(
        &self,
        account_id: &str,
        thread_id: &str,
    ) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_conn_sync(|conn| {
            rtsk::command_palette_queries::get_thread_labels_for_palette(
                conn,
                account_id,
                thread_id,
            )
        })
    }

    /// All user labels across all accounts, with account name in path.
    ///
    /// Each `OptionItem.id` is encoded as `"account_id:kind:label_id"` where
    /// kind is `f` (folder/container) or `t` (tag) so the palette can
    /// construct the correct typed `SidebarSelection` variant.
    pub fn get_all_labels_cross_account(&self) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_conn_sync(rtsk::command_palette_queries::get_all_labels_cross_account)
    }

    /// Check whether an account uses folder-based semantics (Exchange/IMAP/JMAP)
    /// as opposed to tag-based (Gmail). Folder-based providers don't support
    /// Add Label / Remove Label — only Move to Folder.
    pub fn is_folder_based_provider(&self, account_id: &str) -> Result<bool, String> {
        self.with_conn_sync(|conn| rtsk::command_palette_queries::is_folder_based_provider(conn, account_id))
    }
}
