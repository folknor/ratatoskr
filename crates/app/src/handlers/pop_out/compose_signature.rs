use std::sync::Arc;

use iced::Task;

use crate::pop_out::compose::{ComposeMessage, ComposeMode};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::{App, Message};

impl App {
    /// When the user changes the "From" account, update the compose state
    /// and dispatch a signature resolution task for the new account.
    pub(crate) fn handle_compose_from_account_changed(
        &mut self,
        window_id: iced::window::Id,
        msg: ComposeMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };

        // Let the standard update handler process the account change first
        // (sets state.from_account to the new account).
        crate::pop_out::compose::update_compose(state, msg);

        self.resolve_compose_signature(window_id)
    }

    /// Resolve the default signature for the current From account of a
    /// compose window and dispatch a `SignatureResolved` message.
    ///
    /// Used both on initial compose window open and when the user switches
    /// the From account.
    pub(crate) fn resolve_compose_signature(&self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get(&window_id) else {
            return Task::none();
        };

        let Some(ref account) = state.from_account else {
            return Task::none();
        };

        let account_id = account.id.clone();
        let from_email = Some(account.email.clone());
        let is_reply = matches!(
            state.mode,
            ComposeMode::Reply { .. } | ComposeMode::ReplyAll { .. }
        );

        let db = Arc::clone(&self.db);

        Task::perform(
            async move {
                let core_db = db.read_db_state();
                let sig = rtsk::db::queries_extra::db_resolve_signature_for_compose(
                    &core_db, account_id, from_email, is_reply,
                )
                .await?;
                Ok::<_, String>(sig)
            },
            move |result| {
                let (sig_id, sig_html) = match result {
                    Ok(Some(sig)) => (Some(sig.id), Some(sig.body_html)),
                    Ok(None) => (None, None),
                    Err(e) => {
                        log::error!("Signature resolution failed: {e}");
                        (None, None)
                    }
                };
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::SignatureResolved {
                        signature_id: sig_id,
                        signature_html: sig_html,
                    }),
                )
            },
        )
    }
}
