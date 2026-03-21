use std::sync::Arc;

use iced::Task;

use crate::ui::add_account::{AddAccountEvent, AddAccountWizard};
use crate::{App, Message, load_accounts};

impl App {
    pub(crate) fn handle_add_account_event(&mut self, event: AddAccountEvent) -> Task<Message> {
        match event {
            AddAccountEvent::AccountAdded(_account_id) => {
                self.add_account_wizard = None;
                self.no_accounts = false;
                let db = Arc::clone(&self.db);
                self.nav_generation += 1;
                let load_gen = self.nav_generation;
                Task::perform(
                    async move { (load_gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
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
        let used_colors = self.sidebar.accounts.iter()
            .filter_map(|a| a.account_color.clone())
            .collect();
        self.add_account_wizard =
            Some(AddAccountWizard::new_add_account(used_colors, Arc::clone(&self.db)));
        self.show_settings = false;
        Task::none()
    }
}
