use iced::Task;

use crate::command_dispatch::NavigationTarget;
use crate::{App, Message};
use rtsk::scope::ViewScope;

impl App {
    /// Handle navigation to a specific target.
    ///
    /// Sidebar-backed targets update `sidebar.selection` directly.
    /// Non-sidebar targets (chat, search) are stored in `navigation_target`.
    pub(crate) fn handle_navigate_to(&mut self, target: NavigationTarget) -> Task<Message> {
        match target {
            NavigationTarget::Chat { ref email } => {
                return self.enter_chat_view(email.clone());
            }
            NavigationTarget::Sidebar {
                ref selection,
                ref account_id,
            } => {
                // Scope to the correct account if specified
                if let Some(aid) = account_id {
                    self.select_account_by_id(aid);
                }
                self.sidebar.selection = selection.clone();
            }
            NavigationTarget::Search { .. } | NavigationTarget::PinnedSearch { .. } => {
                // These are handled by the search pipeline, not sidebar selection
            }
        }

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
