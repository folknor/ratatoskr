use iced::Task;

use crate::draft_wal;
use crate::pop_out::PopOutWindow;
use crate::{Message, ReadyApp};

use rtsk::db::queries_extra::SaveLocalDraftParams;

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

fn draft_params_from_compose(
    state: &crate::pop_out::compose::ComposeState,
) -> SaveLocalDraftParams {
    let account_id = state
        .from_account
        .as_ref()
        .map(|a| a.id.clone())
        .unwrap_or_default();
    let body_text = state.body.to_html();
    #[allow(clippy::cast_possible_wrap)]
    let sep_idx = state.signature_separator_index.map(|i| i as i64);
    SaveLocalDraftParams {
        id: state.draft_id.clone(),
        account_id,
        to_addresses: tokens_to_csv(&state.to.tokens),
        cc_addresses: tokens_to_csv(&state.cc.tokens),
        bcc_addresses: tokens_to_csv(&state.bcc.tokens),
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
        remote_draft_id: None,
        attachments: None,
        signature_separator_index: sep_idx,
    }
}

impl ReadyApp {
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

    /// Save a single compose window's state by appending to the
    /// UI-side draft WAL. Used by the periodic auto-save timer and
    /// the manual Save button. The append is synchronous and
    /// sub-millisecond (the file is local + we only flush+fsync one
    /// short line); the Service drains the WAL on next boot via
    /// `BootPhase::DrainingDraftWal`.
    ///
    /// Returns `Task::none()` because the WAL append is the
    /// durability point - there is nothing async to await. Phase
    /// 6a-part-2 deletes the synchronous SQLite write that used to
    /// run from this path; the WAL is the only UI write that
    /// survives the lockdown.
    pub(crate) fn save_compose_draft(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        if state.from_account.is_none() {
            return Task::none();
        }
        let params = draft_params_from_compose(state);
        let data_dir = match crate::APP_DATA_DIR.get() {
            Some(d) => d,
            None => {
                log::error!("Failed to auto-save compose draft: APP_DATA_DIR not set");
                return Task::none();
            }
        };
        match draft_wal::append(data_dir, &params) {
            Ok(()) => {
                state.draft_dirty = false;
            }
            Err(e) => {
                log::error!("Failed to append compose draft to WAL: {e}");
            }
        }
        Task::none()
    }

    /// Synchronously save a compose window's draft before the
    /// window is destroyed. Same code path as the auto-save tick;
    /// kept distinct so call sites can express intent.
    ///
    /// Returns `true` if the draft was appended (or didn't need
    /// saving), `false` if the WAL write failed and the draft is
    /// still dirty.
    pub(crate) fn save_compose_draft_sync(&mut self, window_id: iced::window::Id) -> bool {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return true;
        };
        if !state.draft_dirty || state.from_account.is_none() {
            return true;
        }
        let params = draft_params_from_compose(state);
        let data_dir = match crate::APP_DATA_DIR.get() {
            Some(d) => d,
            None => {
                log::error!("Failed to save compose draft on close: APP_DATA_DIR not set");
                return false;
            }
        };
        match draft_wal::append(data_dir, &params) {
            Ok(()) => {
                state.draft_dirty = false;
                true
            }
            Err(e) => {
                log::error!("Failed to append compose draft to WAL on close: {e}");
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
