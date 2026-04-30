use std::sync::Arc;

use iced::Task;

use crate::pop_out::PopOutWindow;
use crate::{App, Message};

use rtsk::db::queries_extra::{db_save_local_draft, db_save_local_draft_sync};

/// Convert token input tokens into a comma-separated string of email addresses.
fn tokens_to_csv(tokens: &[crate::ui::token_input::Token]) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|t| t.email.as_str())
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Data extracted from a `ComposeState` for draft persistence.
struct DraftData {
    draft_id: String,
    account_id: String,
    to_csv: Option<String>,
    cc_csv: Option<String>,
    bcc_csv: Option<String>,
    subject: Option<String>,
    body_html: Option<String>,
    from_email: Option<String>,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    signature_id: Option<String>,
    signature_separator_index: Option<i64>,
}

impl DraftData {
    fn from_compose(state: &crate::pop_out::compose::ComposeState) -> Self {
        let account_id = state
            .from_account
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let body_text = state.body.to_html();
        #[allow(clippy::cast_possible_wrap)]
        let sep_idx = state.signature_separator_index.map(|i| i as i64);
        Self {
            draft_id: state.draft_id.clone(),
            account_id,
            to_csv: tokens_to_csv(&state.to.tokens),
            cc_csv: tokens_to_csv(&state.cc.tokens),
            bcc_csv: tokens_to_csv(&state.bcc.tokens),
            subject: if state.subject.is_empty() {
                None
            } else {
                Some(state.subject.clone())
            },
            body_html: if body_text.trim().is_empty() {
                None
            } else {
                Some(body_text)
            },
            from_email: state.from_account.as_ref().map(|a| a.email.clone()),
            reply_to_message_id: state.reply_message_id.clone(),
            thread_id: state.reply_thread_id.clone(),
            signature_id: state.active_signature_id.clone(),
            signature_separator_index: sep_idx,
        }
    }

}

impl App {
    /// Returns true if at least one compose pop-out window exists.
    /// Computed from `pop_out_windows` - no manual bookkeeping needed.
    pub(crate) fn composer_is_open(&self) -> bool {
        self.pop_out_windows
            .values()
            .any(|w| matches!(w, PopOutWindow::Compose(_)))
    }

    /// Returns true if any compose window has `draft_dirty` set.
    pub(crate) fn has_dirty_compose_drafts(&self) -> bool {
        self.pop_out_windows
            .values()
            .any(|w| matches!(w, PopOutWindow::Compose(s) if s.draft_dirty))
    }

    /// Save a single compose window's state as a local draft (async).
    /// Used by the periodic auto-save timer.
    fn save_compose_draft(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        if state.from_account.is_none() {
            return Task::none();
        }
        state.draft_dirty = false;
        let data = DraftData::from_compose(state);
        let db = Arc::clone(&self.db);

        Task::perform(
            async move {
                let core_db = db.write_db_state();
                db_save_local_draft(
                    &core_db,
                    data.draft_id,
                    data.account_id,
                    data.to_csv,
                    data.cc_csv,
                    data.bcc_csv,
                    data.subject,
                    data.body_html,
                    data.reply_to_message_id,
                    data.thread_id,
                    data.from_email,
                    data.signature_id,
                    None,
                    None,
                    data.signature_separator_index,
                )
                .await
            },
            move |result| {
                if let Err(e) = result {
                    log::error!("Failed to auto-save compose draft: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Synchronously save a compose window's draft before the window is
    /// destroyed. Used on window close where an async Task would race
    /// against `iced::exit()`. A single-row INSERT is sub-millisecond.
    ///
    /// Returns `true` if the draft was saved (or didn't need saving),
    /// `false` if the write failed and the draft is still dirty.
    pub(crate) fn save_compose_draft_sync(&mut self, window_id: iced::window::Id) -> bool {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return true;
        };
        if !state.draft_dirty || state.from_account.is_none() {
            return true;
        }
        let data = DraftData::from_compose(state);

        let result = self.db.write_db_state().with_conn_sync(|conn| {
            db_save_local_draft_sync(
                conn,
                data.draft_id,
                data.account_id,
                data.to_csv,
                data.cc_csv,
                data.bcc_csv,
                data.subject,
                data.body_html,
                data.reply_to_message_id,
                data.thread_id,
                data.from_email,
                data.signature_id,
                None,
                None,
                data.signature_separator_index,
            )
        });
        match result {
            Ok(()) => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.draft_dirty = false;
                }
                true
            }
            Err(e) => {
                log::error!("Failed to save compose draft on close: {e}");
                false
            }
        }
    }

    /// Auto-save all dirty compose drafts. Called from subscription tick.
    pub(crate) fn auto_save_compose_drafts(&mut self) -> Task<Message> {
        let dirty_windows: Vec<iced::window::Id> = self
            .pop_out_windows
            .iter()
            .filter_map(|(&id, w)| {
                if let PopOutWindow::Compose(s) = w {
                    if s.draft_dirty { Some(id) } else { None }
                } else {
                    None
                }
            })
            .collect();

        let mut tasks = Vec::new();
        for win_id in dirty_windows {
            tasks.push(self.save_compose_draft(win_id));
        }
        Task::batch(tasks)
    }
}
