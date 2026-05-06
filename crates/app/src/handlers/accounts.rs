use std::sync::Arc;

use iced::Task;

use crate::ui::add_account::{AddAccountEvent, AddAccountWizard};
use crate::{Message, ReadyApp, load_accounts};

impl ReadyApp {
    pub(crate) fn handle_add_account_event(&mut self, event: AddAccountEvent) -> Task<Message> {
        match event {
            AddAccountEvent::AccountAdded(ref account_id) => {
                log::info!("Account added successfully");
                self.add_account_wizard = None;
                self.no_accounts = false;
                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation.next();
                // Phase 5 task 9b: kick a calendar sync for the new
                // account explicitly. The kick-handler staleness gate
                // would skip it on the first SyncTick (no
                // last_completed entry yet -> stale), but a direct
                // request gives us an awaited terminal completion if
                // any caller wants to chain on it later.
                let calendar_task = self.dispatch_calendar_sync(account_id.clone());
                let load_task = Task::perform(
                    async move { (load_gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                );
                Task::batch([load_task, calendar_task])
            }
            AddAccountEvent::ReauthComplete(account_id) => {
                self.add_account_wizard = None;
                self.status_bar.clear_warning(&account_id);
                let email = self.email_for_account(&account_id);
                self.status_bar
                    .show_confirmation(format!("{email} re-authenticated successfully"));
                Task::none()
            }
            AddAccountEvent::Cancelled => {
                self.add_account_wizard = None;
                Task::none()
            }
        }
    }

    pub(crate) fn handle_open_add_account_wizard(&mut self) -> Task<Message> {
        self.dismiss_overlays();
        let used_colors = self
            .sidebar
            .accounts
            .iter()
            .filter_map(|a| a.account_color.clone())
            .collect();
        self.add_account_wizard = Some(AddAccountWizard::new_add_account(
            used_colors,
            Arc::clone(&self.db),
            self.service_client.clone(),
        ));
        Task::none()
    }

    /// Open a re-auth wizard for an existing account.
    pub(crate) fn handle_open_reauth_wizard(&mut self, account_id: String) -> Task<Message> {
        let email = self.email_for_account(&account_id);
        match AddAccountWizard::new_reauth(
            account_id,
            email.clone(),
            Arc::clone(&self.db),
            self.service_client.clone(),
        ) {
            Ok((wizard, task)) => {
                self.dismiss_overlays();
                self.add_account_wizard = Some(wizard);
                task.map(Message::AddAccount)
            }
            Err(e) => {
                log::error!("Failed to open re-auth wizard for {email}: {e}");
                self.status_bar
                    .show_confirmation(format!("Could not re-authenticate {email}: {e}"));
                Task::none()
            }
        }
    }
}
