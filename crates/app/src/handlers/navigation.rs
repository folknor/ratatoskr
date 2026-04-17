use iced::Task;

use crate::command_dispatch::NavigationTarget;
use crate::{App, Message};
use rtsk::scope::ViewScope;

impl App {
    /// Handle navigation to a specific target.
    ///
    /// Sidebar-backed targets update `sidebar.selection` directly.
    /// Chat targets set `active_chat`.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn handle_navigate_to(&mut self, target: NavigationTarget) -> Task<Message> {
        match target {
            NavigationTarget::Chat { ref email } => {
                return self.enter_chat_view(email.clone());
            }
            NavigationTarget::Sidebar {
                ref selection,
                ref account_id,
            } => {
                if let Some(aid) = account_id {
                    self.select_account_by_id(aid);
                }
                self.sidebar.selection = selection.clone();
            }
            NavigationTarget::Search { .. } | NavigationTarget::PinnedSearch { .. } => {
                // Handled by the search pipeline, not sidebar selection
            }
        }

        self.reset_view_state();
        let token = self.nav_generation.next();
        self.load_threads_for_current_view(token)
    }

    /// Select an account by its ID, updating the sidebar scope.
    fn select_account_by_id(&mut self, account_id: &str) {
        self.sidebar.selected_scope = ViewScope::Account(account_id.to_string());
    }
}
