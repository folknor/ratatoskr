use iced::widget::{Space, column, text};
use iced::{Alignment, Element, Length, Task};
use service_api::{AccountCreateParams, AccountCredentials};

use crate::ui::layout::*;
use crate::ui::widgets;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard,
};
use super::views::{labeled_input, primary_button, titlecase};

impl AddAccountWizard {
    pub(super) fn handle_submit_identity(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if self.identity.name.trim().is_empty() {
            self.error = Some("Please enter an account name.".to_string());
            return (Task::none(), None);
        }
        let Some(client) = self.service_client.as_ref().cloned() else {
            self.error = Some("Service not ready - try again in a moment.".to_string());
            return (Task::none(), None);
        };
        self.step = AddAccountStep::Creating;
        self.error = None;
        let generation = self.generation.next();

        let create_params = self.build_create_params();

        let task = Task::perform(
            async move {
                match client.create_account(create_params).await {
                    Ok(id) => (generation, Ok(id)),
                    Err(e) => (generation, Err(e.to_string())),
                }
            },
            |(g, result)| AddAccountMessage::AccountCreated(g, result),
        );
        (task, None)
    }

    fn build_create_params(&self) -> AccountCreateParams {
        let account_color = self.selected_color_hex();
        let account_name = self.identity.name.trim().to_string();

        // SMTP credentials: separate if the user toggled the checkbox
        let (smtp_user, smtp_pass) = if self.auth_state.use_separate_smtp_credentials {
            (
                Some(self.auth_state.smtp_username.clone()),
                Some(self.auth_state.smtp_password.clone()),
            )
        } else {
            (None, None)
        };

        // Build params based on auth method (password vs OAuth).
        // The Plaintext envelope variant is used in both cases:
        // today's Service handler stores the values verbatim, so the
        // envelope is the boundary where future encryption can land
        // without any caller change.
        if let Some(ref oauth) = self.oauth_success {
            AccountCreateParams {
                email: self.email.clone(),
                provider: self.resolved_provider.clone(),
                display_name: Some(oauth.user_name.clone()),
                account_name,
                account_color,
                auth_method: self.resolved_auth_method.clone(),
                credentials: AccountCredentials::Plaintext {
                    access_token: Some(oauth.access_token.clone()),
                    refresh_token: oauth.refresh_token.clone(),
                    imap_password: None,
                    smtp_password: None,
                },
                token_expires_at: oauth.token_expires_at,
                oauth_provider: Some(oauth.oauth_provider.clone()),
                oauth_client_id: Some(oauth.oauth_client_id.clone()),
                imap_host: None,
                imap_port: None,
                imap_security: None,
                imap_username: None,
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
                smtp_username: None,
                jmap_url: None,
                accept_invalid_certs: false,
            }
        } else {
            let imap_port = self.auth_state.imap_port.parse::<i64>().ok();
            let smtp_port = self.auth_state.smtp_port.parse::<i64>().ok();
            AccountCreateParams {
                email: self.email.clone(),
                provider: self.resolved_provider.clone(),
                display_name: None,
                account_name,
                account_color,
                auth_method: self.resolved_auth_method.clone(),
                credentials: AccountCredentials::Plaintext {
                    access_token: None,
                    refresh_token: None,
                    imap_password: Some(self.auth_state.password.clone()),
                    smtp_password: smtp_pass,
                },
                token_expires_at: None,
                oauth_provider: None,
                oauth_client_id: None,
                imap_host: Some(self.auth_state.imap_host.clone()),
                imap_port,
                imap_security: Some(self.auth_state.imap_security.to_db_string().to_string()),
                imap_username: Some(self.auth_state.username.clone()),
                smtp_host: Some(self.auth_state.smtp_host.clone()),
                smtp_port,
                smtp_security: Some(self.auth_state.smtp_security.to_db_string().to_string()),
                smtp_username: smtp_user,
                jmap_url: None,
                accept_invalid_certs: self.auth_state.accept_invalid_certs,
            }
        }
    }

    pub(super) fn prefill_identity_name(&mut self) {
        if self.identity.name.is_empty() {
            let domain = self.email.split('@').nth(1).unwrap_or("");
            let name = domain.split('.').next().unwrap_or(domain);
            self.identity.name = titlecase(name);
        }
    }

    pub(super) fn view_identity(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        col = col.push(text(&self.email).size(TEXT_LG).style(text::secondary));
        col = col.push(Space::new().height(SPACE_XS));
        col = col.push(labeled_input(
            "Account name",
            "e.g. Work, Personal",
            &self.identity.name,
            AddAccountMessage::AccountNameChanged,
        ));

        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(text("Pick a color").size(TEXT_SM).style(text::secondary));
        col = col.push(widgets::color_palette_grid(
            self.identity.selected_color_index,
            &self.used_colors,
            AddAccountMessage::SelectColor,
            None,
        ));

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(Space::new().height(SPACE_LG));
        col = col.push(primary_button("Done", AddAccountMessage::SubmitIdentity));

        col.into()
    }
}

pub(super) fn view_creating<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Creating account...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        widgets::spinner(24.0),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}
