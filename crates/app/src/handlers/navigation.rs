use iced::Task;

use crate::command_dispatch::NavigationTarget;
use crate::{App, Message};
use rtsk::scope::ViewScope;

impl App {
    /// Handle navigation to a specific target.
    ///
    /// Updates sidebar selection, clears search/pinned search context,
    /// sets the navigation target for view type derivation, and loads
    /// threads for the new view.
    pub(crate) fn handle_navigate_to(&mut self, target: NavigationTarget) -> Task<Message> {
        // Chat targets have their own entry path
        if let NavigationTarget::Chat { ref email } = target {
            return self.enter_chat_view(email.clone());
        }

        // For Label targets, scope to the correct account
        if let NavigationTarget::Label { ref account_id, .. } = target {
            self.select_account_by_id(account_id);
        }

        // Update sidebar selected_label from the target
        self.sidebar.selected_label = target.to_label_id();

        // Full view reset + load
        self.reset_view_state(Some(target));
        let token = self.nav_generation.next();
        self.load_threads_for_current_view(token)
    }

    /// Select an account by its ID, updating the sidebar scope.
    fn select_account_by_id(&mut self, account_id: &str) {
        self.sidebar.selected_scope = ViewScope::Account(account_id.to_string());
    }
}
