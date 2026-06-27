use std::io::Write;
use std::path::PathBuf;

use iced::Task;

use crate::pop_out::PopOutWindow;
use crate::pop_out::compose::ComposeMode;
use crate::{Message, ReadyApp};

use service_api::actions::{SendAttachment, SendIntent};

impl ReadyApp {
    /// Build a MIME message from the compose state, save it to the draft row
    /// as base64url in the `attachments` column, mark the draft `'queued'`,
    /// Validate compose state, build a SendRequest, and dispatch to the
    /// action service. The compose window stays open with a "Sending..." status
    /// until SendCompleted arrives.
    pub(crate) fn handle_compose_send(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };

        // Prevent double-send
        if state.sending {
            return Task::none();
        }

        // Validate recipients
        let has_recipients = !state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty();
        if !has_recipients {
            state.status = Some("Add at least one recipient".to_string());
            return Task::none();
        }

        // Build SendRequest from compose state
        let account_info = match state.from_account.as_ref() {
            Some(a) => a.clone(),
            None => {
                state.status = Some("No sending account selected".to_string());
                return Task::none();
            }
        };

        let from = if let Some(ref name) = account_info.display_name {
            format!("{name} <{}>", account_info.email)
        } else {
            account_info.email.clone()
        };

        let to: Vec<String> = state.to.tokens.iter().map(|t| t.email.clone()).collect();
        let cc: Vec<String> = state.cc.tokens.iter().map(|t| t.email.clone()).collect();
        let bcc: Vec<String> = state.bcc.tokens.iter().map(|t| t.email.clone()).collect();

        let subject = if state.subject.is_empty() {
            None
        } else {
            Some(state.subject.clone())
        };

        let body_html = state.body.to_html();
        let body_text = state.body.document.flattened_text();

        let attachments: Vec<SendAttachment> = state
            .attachments
            .iter()
            .map(|a| SendAttachment {
                filename: a.name.clone(),
                mime_type: a.mime_type.clone(),
                data: a.data.as_ref().clone().into(),
                content_id: None,
            })
            .collect();

        // Reuse draft_id on retry so the action updates the existing
        // 'failed' row instead of creating a new one.
        let draft_id = state
            .send_draft_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();

        let send_req = service_api::actions::SendRequest {
            draft_id,
            account_id: account_info.id.clone(),
            from,
            to,
            cc,
            bcc,
            subject,
            body_html,
            body_text,
            attachments,
            in_reply_to: state.reply_message_id.clone(),
            references: state.reply_message_id.clone(),
            thread_id: state.reply_thread_id.clone(),
            source_message_id: state.reply_source_message_id.clone(),
            intent: send_intent_from_mode(&state.mode),
        };

        // Set sending state and dispatch
        state.sending = true;
        state.status = Some("Sending\u{2026}".to_string());

        self.dispatch_send(window_id, send_req)
    }

    /// Dispatch send through the Service via `action.send` IPC.
    ///
    /// Phase 2 task 13. UI writes attachment bytes to
    /// `<app_data>/staging/<send_id>/<index>.bin` (UI-owned), then
    /// issues the IPC. Service handler verifies SHA-256, atomically
    /// renames into the Service-owned vault, journals the send, and
    /// returns `SendAck`. The compose window stays in "sending"
    /// until the eventual `ActionCompleted` notification (matching
    /// `send_id`) fires `Message::SendCompleted`.
    fn dispatch_send(
        &mut self,
        window_id: iced::window::Id,
        request: service_api::actions::SendRequest,
    ) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) {
                state.sending = false;
                state.status = Some("Send unavailable \u{2014} Service not connected".to_string());
            }
            return Task::none();
        };
        let app_data_dir = crate::APP_DATA_DIR
            .get()
            .expect("APP_DATA_DIR set before send")
            .clone();

        let send_id = service_api::PlanId::new_v7();
        // Stage every attachment to disk and build the wire request.
        // Returns the wire request on success, an error string on
        // first per-attachment failure (which restores the compose
        // window state via the SendCompleted message).
        let wire = match stage_and_build_wire(&app_data_dir, send_id, request) {
            Ok(w) => w,
            Err(error) => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status = Some(format!("Send failed: {error}"));
                }
                return Task::none();
            }
        };

        // Track the in-flight send so the ActionCompleted notification
        // can fire SendCompleted against this window.
        self.in_flight_sends.insert(send_id, window_id);

        let staging_dir = app_data_dir.join("staging").join(send_id.to_string());
        Task::perform(
            async move {
                let result = client.send_email(wire).await;
                (window_id, send_id, staging_dir, result)
            },
            move |(window_id, send_id, staging_dir, result)| {
                // Cleanup the staging directory regardless of ack
                // outcome: on success the bytes are now in the
                // Service vault, on failure they're not in the
                // vault and the user will retry from compose state.
                let _ = std::fs::remove_dir_all(&staging_dir);
                let _ = send_id;
                match result {
                    // Ack received: the Service journaled the send. The
                    // worker will dispatch SMTP and emit ActionCompleted;
                    // the in_flight_sends entry routes that to the
                    // compose window. Nothing to do at the UI layer
                    // here.
                    Ok(_ack) => Message::Noop,
                    // IPC failure (ServiceCrashed / Timeout / handler
                    // returned ServiceError). Surface as a send
                    // failure on the compose window. The orphan
                    // in_flight_sends entry stays - if a late
                    // ActionCompleted ever arrives, the SendCompleted
                    // handler will no-op against the closed window.
                    Err(error) => Message::SendCompleted {
                        window_id,
                        outcome: service_api::actions::ActionOutcome::Failed {
                            error: service_api::actions::ActionError::remote(format!("{error}")),
                        },
                    },
                }
            },
        )
    }

    /// Handle send completion: close compose on success, restore on failure.
    pub(crate) fn handle_send_completed(
        &mut self,
        window_id: iced::window::Id,
        outcome: &service_api::actions::ActionOutcome,
    ) -> Task<Message> {
        match outcome {
            service_api::actions::ActionOutcome::Success
            | service_api::actions::ActionOutcome::NoOp => {
                self.pop_out_windows.remove(&window_id);
                self.status_bar
                    .show_confirmation("Message sent".to_string());
                iced::window::close(window_id)
            }
            // LocalOnly should not occur for send (send uses Failed for all
            // failures), but handle it defensively as failure for safety.
            service_api::actions::ActionOutcome::Failed { error }
            | service_api::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status = Some(format!("Send failed: {}", error.user_message()));
                }
                Task::none()
            }
        }
    }
}

/// Stage every attachment under `<app_data>/staging/<send_id>/`,
/// SHA-256 the bytes, and assemble a `SendWireRequest` referencing
/// the staged paths. Removes the staging directory and returns the
/// first error if any per-attachment write fails - the caller
/// surfaces the error in the compose status line.
///
/// Synchronous. Attachments are typically small (the dev fixtures
/// cap at ~50 MB) and the staging write happens on the iced runtime
/// only when the user clicks Send; the brief blocking is
/// indistinguishable from the existing in-process MIME build.
fn stage_and_build_wire(
    app_data_dir: &std::path::Path,
    send_id: service_api::PlanId,
    request: service_api::actions::SendRequest,
) -> Result<service_api::SendWireRequest, String> {
    let staging_dir: PathBuf = app_data_dir.join("staging").join(send_id.to_string());
    if let Err(error) = std::fs::create_dir_all(&staging_dir) {
        return Err(format!(
            "create staging dir {}: {error}",
            staging_dir.display()
        ));
    }

    let mut wire_attachments = Vec::with_capacity(request.attachments.len());
    for (index, att) in request.attachments.into_iter().enumerate() {
        let relative = format!("{index}.bin");
        let staged_path = staging_dir.join(&relative);
        if let Err(error) = write_atomic(&staged_path, &att.data) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(format!("write staging {}: {error}", staged_path.display()));
        }
        let content_hash = *rtsk::blob_hash::BlobHash::hash(&att.data).as_bytes();
        let size = att.data.len() as u64;
        wire_attachments.push(service_api::SendWireAttachment {
            source: service_api::SendAttachmentSource::StagingFile {
                relative_path: relative,
                content_hash,
            },
            size,
            mime: att.mime_type,
            filename: att.filename,
            content_id: att.content_id,
        });
    }

    Ok(service_api::SendWireRequest {
        send_id,
        from_account_id: request.account_id,
        message: service_api::SendWireMessage {
            draft_id: request.draft_id,
            from: request.from,
            to: request.to,
            cc: request.cc,
            bcc: request.bcc,
            subject: request.subject,
            body_html: request.body_html,
            body_text: request.body_text,
            in_reply_to: request.in_reply_to,
            references: request.references,
            thread_id: request.thread_id,
            source_message_id: request.source_message_id,
            intent: wire_send_intent(request.intent),
            scheduled_at: None,
        },
        attachments: wire_attachments,
        scheduled_at: None,
    })
}

fn wire_send_intent(intent: SendIntent) -> service_api::SendIntent {
    match intent {
        SendIntent::New => service_api::SendIntent::New,
        SendIntent::Reply => service_api::SendIntent::Reply,
        SendIntent::Forward => service_api::SendIntent::Forward,
    }
}

/// Write bytes to a file. Uses `File::create` + `write_all` rather
/// than tempfile-rename: the staging directory is freshly created
/// for each send_id, so there is no concurrent-writer concern.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn send_intent_from_mode(mode: &ComposeMode) -> SendIntent {
    match mode {
        ComposeMode::New => SendIntent::New,
        ComposeMode::Reply { .. } | ComposeMode::ReplyAll { .. } => SendIntent::Reply,
        ComposeMode::Forward { .. } => SendIntent::Forward,
    }
}
