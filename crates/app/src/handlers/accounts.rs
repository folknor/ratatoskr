use std::sync::Arc;

use iced::Task;

use crate::ui::add_account::{AddAccountEvent, AddAccountWizard};
use crate::{App, Message, load_accounts};

impl App {
    pub(crate) fn handle_add_account_event(&mut self, event: AddAccountEvent) -> Task<Message> {
        match event {
            AddAccountEvent::AccountAdded(ref _account_id) => {
                log::info!("Account added successfully");
                self.add_account_wizard = None;
                self.no_accounts = false;
                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation.next();
                Task::perform(
                    async move { (load_gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
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
                if !self.no_accounts {
                    self.add_account_wizard = None;
                }
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
        ));
        Task::none()
    }

    /// Open a re-auth wizard for an existing account.
    pub(crate) fn handle_open_reauth_wizard(&mut self, account_id: String) -> Task<Message> {
        let email = self.email_for_account(&account_id);
        match AddAccountWizard::new_reauth(account_id, email.clone(), Arc::clone(&self.db)) {
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
