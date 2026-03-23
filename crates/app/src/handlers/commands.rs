use iced::Task;

use std::sync::Arc;

use crate::command_dispatch::{self, EmailAction};
use crate::db::Db;
use crate::{APP_DATA_DIR, App, Message};
use ratatoskr_command_palette::{CommandArgs, CommandId, KeyBinding, OptionItem};
use ratatoskr_core::actions::ActionOutcome;

impl App {
    /// Save keybinding overrides to disk. Call this after any mutation
    /// to `self.binding_table` overrides (`set_override`, `unbind`,
    /// `remove_override`, `reset_all`).
    pub(crate) fn save_keybinding_overrides(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("keybindings.json");
        if let Err(e) = self.binding_table.save_overrides(&path) {
            eprintln!("warning: failed to save keybinding overrides: {e}");
        }
    }

    /// Set a keybinding override and persist to disk.
    /// Returns `Err(conflicting_id)` if the binding conflicts.
    pub(crate) fn set_keybinding(
        &mut self,
        id: CommandId,
        binding: KeyBinding,
    ) -> Result<(), CommandId> {
        self.binding_table.set_override(id, binding)?;
        self.save_keybinding_overrides();
        Ok(())
    }

    /// Unbind a command (explicit, no fallback to default) and persist.
    pub(crate) fn unbind_keybinding(&mut self, id: CommandId) {
        self.binding_table.unbind(id);
        self.save_keybinding_overrides();
    }

    /// Remove a keybinding override (revert to default) and persist.
    pub(crate) fn remove_keybinding_override(&mut self, id: CommandId) {
        self.binding_table.remove_override(id);
        self.save_keybinding_overrides();
    }

    /// Reset all keybinding overrides to defaults and persist.
    pub(crate) fn reset_all_keybindings(&mut self) {
        self.binding_table.reset_all();
        self.save_keybinding_overrides();
    }

    pub(crate) fn handle_execute_command(&mut self, id: CommandId) -> Task<Message> {
        log::debug!("Executing command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_command(id, self) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    pub(crate) fn handle_execute_parameterized(
        &mut self,
        id: CommandId,
        args: CommandArgs,
    ) -> Task<Message> {
        log::debug!("Executing parameterized command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_parameterized(id, args) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    /// Save usage counts to disk.
    fn save_usage_counts(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("command_usage.json");
        let map = self.registry.usage.to_map();
        match serde_json::to_string(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("Failed to save usage counts: {e}");
                }
            }
            Err(e) => log::warn!("Failed to serialize usage counts: {e}"),
        }
    }

    pub(crate) fn handle_email_action(
        &mut self,
        action: EmailAction,
    ) -> Task<Message> {
        let selection_count = self.thread_list.selection_count();

        // Archive goes through the action service (Phase 1).
        // All other actions use the legacy inline path until Phase 2.
        if matches!(action, EmailAction::Archive) {
            let selected_threads: Vec<(String, String)> = self
                .thread_list
                .selected_indices()
                .iter()
                .filter_map(|&i| self.thread_list.threads.get(i))
                .map(|t| (t.account_id.clone(), t.id.clone()))
                .collect();

            if selected_threads.is_empty() {
                return Task::none();
            }

            let archive_task = self.dispatch_archive(selected_threads);
            let advance_task = self.handle_thread_list(
                crate::ui::thread_list::ThreadListMessage::AutoAdvance,
            );
            return Task::batch([archive_task, advance_task]);
        }

        let confirmation = match &action {
            EmailAction::Archive => unreachable!("handled above"),
            EmailAction::Trash => Some("Moved to Trash"),
            EmailAction::PermanentDelete => Some("Permanently deleted"),
            EmailAction::ToggleSpam => Some("Spam status toggled"),
            EmailAction::ToggleRead => Some("Read status toggled"),
            EmailAction::ToggleStar => Some("Star toggled"),
            EmailAction::TogglePin => Some("Pin toggled"),
            EmailAction::ToggleMute => Some("Mute toggled"),
            EmailAction::Unsubscribe => Some("Unsubscribed"),
            EmailAction::MoveToFolder { .. } => Some("Moved to folder"),
            EmailAction::AddLabel { .. } => Some("Label applied"),
            EmailAction::RemoveLabel { .. } => Some("Label removed"),
            EmailAction::Snooze { .. } => Some("Snoozed"),
        };
        if let Some(msg) = confirmation {
            let display = if selection_count > 1 {
                format!("{msg} ({selection_count} threads)")
            } else {
                msg.to_string()
            };
            self.status_bar.show_confirmation(display);
        }

        // Collect selected thread info before any mutation.
        let selected_threads: Vec<(String, String)> = self
            .thread_list
            .selected_indices()
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if selected_threads.is_empty() {
            return Task::none();
        }

        // Destructive actions remove the thread from the current view
        // — perform DB update + trigger auto-advance.
        let removes_from_view = matches!(
            action,
            EmailAction::Trash
                | EmailAction::PermanentDelete
                | EmailAction::ToggleSpam
                | EmailAction::MoveToFolder { .. }
                | EmailAction::Snooze { .. }
        );

        if removes_from_view {
            let db_task = self.dispatch_email_db_action(&action, &selected_threads);
            let advance_task = self.handle_thread_list(
                crate::ui::thread_list::ThreadListMessage::AutoAdvance,
            );
            return Task::batch([db_task, advance_task]);
        }

        // Non-destructive actions: dispatch DB update in place.
        match action {
            EmailAction::ToggleStar => self.toggle_star_selected_threads(),
            EmailAction::AddLabel { label_id } => {
                self.apply_label_to_selected_threads(&label_id)
            }
            EmailAction::RemoveLabel { label_id } => {
                self.remove_label_from_selected_threads(&label_id)
            }
            EmailAction::ToggleRead => self.toggle_bool_selected_threads(
                "is_read",
                &selected_threads,
                |t| &mut t.is_read,
            ),
            EmailAction::TogglePin => self.toggle_bool_selected_threads(
                "is_pinned",
                &selected_threads,
                |t| &mut t.is_pinned,
            ),
            EmailAction::ToggleMute => self.toggle_bool_selected_threads(
                "is_muted",
                &selected_threads,
                |t| &mut t.is_muted,
            ),
            _ => Task::none(),
        }
    }

    /// Archive via the action service (Phase 1).
    fn dispatch_archive(&mut self, threads: Vec<(String, String)>) -> Task<Message> {
        let Some(ref action_ctx) = self.action_ctx else {
            self.status_bar.show_confirmation(
                "Archive unavailable \u{2014} action service not initialized".to_string(),
            );
            return Task::none();
        };

        let ctx = action_ctx.clone();
        Task::perform(
            async move {
                let mut outcomes = Vec::with_capacity(threads.len());
                for (account_id, thread_id) in &threads {
                    outcomes.push(
                        ratatoskr_core::actions::archive(&ctx, account_id, thread_id).await,
                    );
                }
                outcomes
            },
            Message::ArchiveCompleted,
        )
    }

    /// Handle archive completion — map outcomes to user feedback.
    pub(crate) fn handle_archive_completed(
        &mut self,
        outcomes: &[ActionOutcome],
    ) -> Task<Message> {
        let any_failed = outcomes.iter().any(ActionOutcome::is_failed);
        let any_local_only = outcomes.iter().any(ActionOutcome::is_local_only);

        if any_failed {
            let errors: Vec<&str> = outcomes
                .iter()
                .filter_map(|o| match o {
                    ActionOutcome::Failed { error } => Some(error.as_str()),
                    _ => None,
                })
                .collect();
            self.status_bar
                .show_confirmation(format!("Archive failed: {}", errors.join("; ")));
        } else if any_local_only {
            self.status_bar.show_confirmation(
                "Archived locally \u{2014} sync may revert this".to_string(),
            );
        } else {
            self.status_bar
                .show_confirmation("Archived".to_string());
        }
        Task::none()
    }

    /// Dispatch the DB operation for an email action (async, fire-and-forget).
    /// Legacy path — actions are migrated to the action service in Phase 2.
    fn dispatch_email_db_action(
        &self,
        action: &EmailAction,
        threads: &[(String, String)],
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let threads = threads.to_vec();
        let action = action.clone();

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    use ratatoskr_core::email_actions::{insert_label, remove_label};

                    for (account_id, thread_id) in &threads {
                        match &action {
                            EmailAction::Archive => {
                                // Archive is handled by the action service — should
                                // not reach here. If it does, no-op rather than
                                // silently doing local-only work.
                                log::error!("Archive reached legacy dispatch path — this is a bug");
                            }
                            EmailAction::Trash => {
                                remove_label(conn, account_id, thread_id, "INBOX")?;
                                insert_label(conn, account_id, thread_id, "TRASH")?;
                            }
                            EmailAction::PermanentDelete => {
                                ratatoskr_core::db::queries::delete_thread(
                                    conn, account_id, thread_id,
                                )?;
                            }
                            EmailAction::ToggleSpam => {
                                remove_label(conn, account_id, thread_id, "INBOX")?;
                                insert_label(conn, account_id, thread_id, "SPAM")?;
                            }
                            EmailAction::MoveToFolder { folder_id } => {
                                // Remove from current folder (INBOX), add to target.
                                remove_label(conn, account_id, thread_id, "INBOX")?;
                                insert_label(conn, account_id, thread_id, folder_id)?;
                            }
                            EmailAction::Snooze { until } => {
                                remove_label(conn, account_id, thread_id, "INBOX")?;
                                conn.execute(
                                    "UPDATE threads SET is_snoozed = 1, snooze_until = ?3 \
                                     WHERE account_id = ?1 AND id = ?2",
                                    rusqlite::params![account_id, thread_id, until],
                                )
                                .map_err(|e| format!("snooze: {e}"))?;
                            }
                            // Non-destructive actions handled separately.
                            _ => {}
                        }
                    }
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Email action DB error: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Toggle a boolean field (is_read, is_pinned, is_muted) on selected threads.
    /// Updates local UI optimistically, then persists to DB.
    fn toggle_bool_selected_threads(
        &mut self,
        field: &'static str,
        threads: &[(String, String)],
        get_field: fn(&mut crate::db::Thread) -> &mut bool,
    ) -> Task<Message> {
        // Optimistic UI toggle
        let indices = self.thread_list.selected_indices();
        let mut toggled: Vec<(String, String, bool)> = Vec::new();
        for &i in &indices {
            if let Some(t) = self.thread_list.threads.get_mut(i) {
                let val = get_field(t);
                *val = !*val;
                let new_val = *val;
                toggled.push((t.account_id.clone(), t.id.clone(), new_val));
            }
        }

        if toggled.is_empty() {
            return Task::none();
        }

        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id, new_value) in &toggled {
                        match field {
                            "is_read" => ratatoskr_core::db::queries::set_thread_read(
                                conn, account_id, thread_id, *new_value,
                            )?,
                            "is_pinned" => ratatoskr_core::db::queries::set_thread_pinned(
                                conn, account_id, thread_id, *new_value,
                            )?,
                            "is_muted" => ratatoskr_core::db::queries::set_thread_muted(
                                conn, account_id, thread_id, *new_value,
                            )?,
                            _ => {}
                        }
                    }
                    Ok(())
                })
                .await
            },
            move |result| {
                if let Err(e) = result {
                    log::error!("Toggle {field} DB error: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Apply a label to all selected threads.
    /// 1. Local DB: insert thread_labels entries (instant UI feedback)
    /// 2. Provider write-back: call add_tag (container) or apply_category (tag)
    fn apply_label_to_selected_threads(&self, label_id: &str) -> Task<Message> {
        let indices = self.thread_list.selected_indices();
        let threads: Vec<(String, String)> = indices
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if threads.is_empty() {
            return Task::none();
        }

        let db = std::sync::Arc::clone(&self.db);
        let lid = label_id.to_string();
        let encryption_key = self.encryption_key;

        Task::perform(
            async move {
                // 1. Local DB write
                let threads_clone = threads.clone();
                let lid_clone = lid.clone();
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id) in &threads_clone {
                        conn.execute(
                            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
                             VALUES (?1, ?2, ?3)",
                            rusqlite::params![account_id, thread_id, lid_clone],
                        )
                        .map_err(|e| format!("apply label: {e}"))?;
                    }
                    Ok(())
                })
                .await?;

                // 2. Provider write-back (best-effort)
                if let Some(key) = encryption_key {
                    let label_info = db.with_conn({
                        let lid = lid.clone();
                        move |conn| {
                            conn.query_row(
                                "SELECT name, label_kind FROM labels WHERE id = ?1 LIMIT 1",
                                rusqlite::params![lid],
                                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                            )
                            .map_err(|e| e.to_string())
                        }
                    })
                    .await;

                    if let Ok((label_name, label_kind)) = label_info {
                        for (account_id, thread_id) in &threads {
                            if let Err(e) = provider_label_write_back(
                                &db, account_id, thread_id, &lid, &label_name, &label_kind, key, true,
                            ).await {
                                log::warn!("Provider write-back failed for {account_id}/{thread_id}: {e}");
                            }
                        }
                    }
                }
                Ok::<(), String>(())
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to apply label: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Remove a label from all selected threads.
    /// 1. Local DB: delete thread_labels entries
    /// 2. Provider write-back: call remove_tag (container) or remove_category (tag)
    fn remove_label_from_selected_threads(&self, label_id: &str) -> Task<Message> {
        let indices = self.thread_list.selected_indices();
        let threads: Vec<(String, String)> = indices
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if threads.is_empty() {
            return Task::none();
        }

        let db = std::sync::Arc::clone(&self.db);
        let lid = label_id.to_string();
        let encryption_key = self.encryption_key;

        Task::perform(
            async move {
                // 1. Local DB write
                let threads_clone = threads.clone();
                let lid_clone = lid.clone();
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id) in &threads_clone {
                        conn.execute(
                            "DELETE FROM thread_labels \
                             WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
                            rusqlite::params![account_id, thread_id, lid_clone],
                        )
                        .map_err(|e| format!("remove label: {e}"))?;
                    }
                    Ok(())
                })
                .await?;

                // 2. Provider write-back (best-effort)
                if let Some(key) = encryption_key {
                    let label_info = db.with_conn({
                        let lid = lid.clone();
                        move |conn| {
                            conn.query_row(
                                "SELECT name, label_kind FROM labels WHERE id = ?1 LIMIT 1",
                                rusqlite::params![lid],
                                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                            )
                            .map_err(|e| e.to_string())
                        }
                    })
                    .await;

                    if let Ok((label_name, label_kind)) = label_info {
                        for (account_id, thread_id) in &threads {
                            if let Err(e) = provider_label_write_back(
                                &db, account_id, thread_id, &lid, &label_name, &label_kind, key, false,
                            ).await {
                                log::warn!("Provider write-back failed for {account_id}/{thread_id}: {e}");
                            }
                        }
                    }
                }
                Ok::<(), String>(())
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to remove label: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Toggle the star flag on all selected threads.
    fn toggle_star_selected_threads(&mut self) -> Task<Message> {
        let indices = self.thread_list.selected_indices();
        let threads: Vec<(String, String, bool)> = indices
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone(), t.is_starred))
            .collect();

        if threads.is_empty() {
            return Task::none();
        }

        // Toggle local UI state immediately
        for &i in &indices {
            if let Some(t) = self.thread_list.threads.get_mut(i) {
                t.is_starred = !t.is_starred;
            }
        }
        // Also update reading pane if it's showing one of these threads
        for (_, tid, was_starred) in &threads {
            self.reading_pane.update_star(tid, !was_starred);
        }

        let db = std::sync::Arc::clone(&self.db);

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id, was_starred) in &threads {
                        ratatoskr_core::db::queries::set_thread_starred(
                            conn, account_id, thread_id, !was_starred,
                        )?;
                    }
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to toggle star: {e}");
                }
                Message::Noop
            },
        )
    }
}

/// Dispatch a provider-side label apply or remove for a single thread.
///
/// Creates a minimal `ProviderCtx` with a real `DbState` (needed for token
/// refresh). Body store, inline images, and search are not used by label
/// operations — dummy/empty state is passed. The `add` parameter controls
/// whether this is an apply (true) or remove (false).
#[allow(clippy::too_many_arguments)]
async fn provider_label_write_back(
    db: &Arc<Db>,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
    label_name: &str,
    label_kind: &str,
    encryption_key: [u8; 32],
    add: bool,
) -> Result<(), String> {
    let provider = super::provider::create_provider(db, account_id, encryption_key).await?;
    let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());

    // For label write-back we only need db + account_id on the ctx.
    // Body store, inline images, search, and progress are unused by
    // add_tag / remove_tag / apply_category / remove_category.
    // We pass the real db but use ProgressReporter::noop for the rest.
    let data_dir = crate::APP_DATA_DIR.get().ok_or("APP_DATA_DIR not set")?;
    let body_store = ratatoskr_core::body_store::BodyStoreState::init(data_dir)
        .map_err(|e| format!("body store init: {e}"))?;
    let search = ratatoskr_core::search::SearchState::init(data_dir)
        .map_err(|e| format!("search init: {e}"))?;
    let inline_images = ratatoskr_stores::inline_image_store::InlineImageStoreState::init(data_dir)
        .map_err(|e| format!("inline image init: {e}"))?;

    let ctx = ratatoskr_provider_utils::types::ProviderCtx {
        account_id,
        db: &core_db,
        body_store: &body_store,
        inline_images: &inline_images,
        search: &search,
        progress: &ratatoskr_core::progress::NoopProgressReporter,
    };

    let result = if add {
        if label_kind == "tag" {
            provider.apply_category(&ctx, thread_id, label_name).await
        } else {
            provider.add_tag(&ctx, thread_id, label_id).await
        }
    } else if label_kind == "tag" {
        provider.remove_category(&ctx, thread_id, label_name).await
    } else {
        provider.remove_tag(&ctx, thread_id, label_id).await
    };

    result.map_err(|e| e.to_string())
}

/// Build typed `CommandArgs` from the selected option item.
///
/// Maps each parameterized `CommandId` to its corresponding `CommandArgs`
/// variant, extracting the item's ID (and for cross-account commands,
/// splitting the `"account_id:label_id"` encoding).
pub(crate) fn build_command_args(command_id: CommandId, item: &OptionItem) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => Some(CommandArgs::MoveToFolder {
            folder_id: item.id.clone(),
        }),
        CommandId::EmailAddLabel => Some(CommandArgs::AddLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailRemoveLabel => Some(CommandArgs::RemoveLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailSnooze => {
            // DateTime picker returns a stringified unix timestamp
            item.id
                .parse::<i64>()
                .ok()
                .map(|ts| CommandArgs::Snooze { until: ts })
        }
        CommandId::NavigateToLabel => {
            let (account_id, label_id) = split_cross_account_id(&item.id)?;
            Some(CommandArgs::NavigateToLabel {
                label_id,
                account_id,
            })
        }
        _ => None,
    }
}

/// Build `CommandArgs` from free text input for Text-param commands.
pub(crate) fn build_command_args_from_text(
    command_id: CommandId,
    text: &str,
) -> Option<CommandArgs> {
    match command_id {
        CommandId::SmartFolderSave => Some(CommandArgs::SmartFolderSave {
            name: text.to_string(),
        }),
        _ => None,
    }
}

/// Split a cross-account encoded ID ("account_id:label_id") into its parts.
fn split_cross_account_id(encoded: &str) -> Option<(String, String)> {
    let colon_pos = encoded.find(':')?;
    let account_id = encoded[..colon_pos].to_string();
    let label_id = encoded[colon_pos + 1..].to_string();
    if account_id.is_empty() || label_id.is_empty() {
        return None;
    }
    Some((account_id, label_id))
}
