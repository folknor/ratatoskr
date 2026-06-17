use super::connection::Db;

// Palette query methods.

impl Db {
    /// User-visible folders for an account, excluding system folders.
    /// Returns `OptionItem`s for the palette's ListPicker stage 2.
    pub fn get_user_folders_for_palette(
        &self,
        account_id: &str,
    ) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_read_sync(|conn| {
            rtsk::command_palette_queries::get_user_folders_for_palette(conn, account_id)
        })
    }

    /// All user-visible label groups.
    pub fn get_label_groups_for_palette(&self) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_read_sync(rtsk::command_palette_queries::get_label_groups_for_palette)
    }

    /// Label groups currently rendered for a specific thread.
    pub fn get_thread_label_groups_for_palette(
        &self,
        account_id: &str,
        thread_id: &str,
    ) -> Result<Vec<cmdk::OptionItem>, String> {
        self.with_read_sync(|conn| {
            rtsk::command_palette_queries::get_thread_label_groups_for_palette(
                conn, account_id, thread_id,
            )
        })
    }
}
