use iced::Task;

use crate::command_dispatch::NavigationTarget;
use crate::{App, Message};

impl App {
    /// Handle navigation to a specific target.
    ///
    /// Updates sidebar selection, clears search/pinned search context,
    /// sets the navigation target for view type derivation, and loads
    /// threads for the new view.
    pub(crate) fn handle_navigate_to(
        &mut self,
        target: NavigationTarget,
    ) -> Task<Message> {
        // Clear search and pinned search state on any navigation
        self.clear_search_state();
        self.clear_pinned_search_context();

        // For Label targets, scope to the correct account
        if let NavigationTarget::Label { ref account_id, .. } = target {
            self.select_account_by_id(account_id);
        }

        // Update sidebar selected_label from the target
        self.sidebar.selected_label = target.to_label_id();

        // Store the navigation target for view type derivation
        self.navigation_target = Some(target);

        // Reset thread selection and bump generations
        self.thread_list.selected_thread = None;
        self.reading_pane.set_thread(None);
        self.nav_generation += 1;
        self.thread_generation += 1;

        // Update thread list header context
        self.update_thread_list_context_from_sidebar();

        // Load threads for the new view
        self.load_threads_for_current_view()
    }

    /// Select an account by its ID, updating the sidebar scope.
    fn select_account_by_id(&mut self, account_id: &str) {
        let idx = self
            .sidebar
            .accounts
            .iter()
            .position(|a| a.id == account_id);
        if let Some(idx) = idx {
            self.sidebar.selected_account = Some(idx);
        }
    }
}
