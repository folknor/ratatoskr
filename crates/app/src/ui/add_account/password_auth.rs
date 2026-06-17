use iced::widget::{Space, column, row, scrollable, text};
use iced::{Alignment, Element, Length, Task};

use crate::font;
use crate::ui::layout::*;
use crate::ui::widgets;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard, SecurityOption,
};
use super::views::{
    ghost_button, labeled_input, primary_button, security_selector, server_port_row,
};

impl AddAccountWizard {
    pub(super) fn handle_submit_credentials(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if self.auth_state.username.trim().is_empty() {
            self.error = Some("Username is required.".to_string());
            return (Task::none(), None);
        }
        if self.auth_state.password.is_empty() {
            self.error = Some("Password is required.".to_string());
            return (Task::none(), None);
        }

        // Wire credential validation - test IMAP connection
        self.step = AddAccountStep::Validating;
        self.error = None;
        let generation = self.generation.next();

        let host = self.auth_state.imap_host.clone();
        let port_str = self.auth_state.imap_port.clone();
        let security = self.auth_state.imap_security;
        let username = self.auth_state.username.clone();
        let password = self.auth_state.password.clone();
        let accept_invalid_certs = self.auth_state.accept_invalid_certs;

        let task = Task::perform(
            async move {
                let result = validate_imap_connection(
                    &host,
                    &port_str,
                    security,
                    &username,
                    &password,
                    accept_invalid_certs,
                )
                .await;
                (generation, result)
            },
            |(g, result)| AddAccountMessage::ValidationComplete(g, result),
        );
        (task, None)
    }

    pub(super) fn view_password_auth(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        let heading = if self.reauth_account_id.is_some() {
            "Re-authenticate"
        } else {
            "Sign In"
        };

        col = col.push(
            text(heading)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );

        // Show the account email for re-auth context
        if self.reauth_account_id.is_some() {
            col = col.push(text(&self.email).size(TEXT_LG).style(text::secondary));
        }

        // IMAP section
        col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "imap.example.com",
            &self.auth_state.imap_host,
            "993",
            &self.auth_state.imap_port,
            AddAccountMessage::AuthImapHostChanged,
            AddAccountMessage::AuthImapPortChanged,
        ));
        col = col.push(security_selector(
            self.auth_state.imap_security,
            AddAccountMessage::AuthImapSecurityChanged,
        ));
        col = col.push(labeled_input(
            "Username",
            "alice@example.com",
            &self.auth_state.username,
            AddAccountMessage::UsernameChanged,
        ));
        // INTENTIONAL: Password field is plaintext - no .secure(true).
        // This is a deliberate product decision per problem-statement.md.
        // Users need to see what they type for app-specific passwords.
        col = col.push(labeled_input(
            "Password",
            "",
            &self.auth_state.password,
            AddAccountMessage::PasswordChanged,
        ));

        // SMTP section
        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "smtp.example.com",
            &self.auth_state.smtp_host,
            "587",
            &self.auth_state.smtp_port,
            AddAccountMessage::AuthSmtpHostChanged,
            AddAccountMessage::AuthSmtpPortChanged,
        ));
        col = col.push(security_selector(
            self.auth_state.smtp_security,
            AddAccountMessage::AuthSmtpSecurityChanged,
        ));

        col = col.push(self.view_password_auth_options());

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(primary_button(
            "Sign In",
            AddAccountMessage::SubmitCredentials,
        ));
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        scrollable(col).spacing(SCROLLBAR_SPACING).into()
    }

    fn view_password_auth_options(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_SM).width(Length::Fill);

        col = col.push(
            row![
                iced::widget::checkbox(self.auth_state.use_separate_smtp_credentials)
                    .on_toggle(AddAccountMessage::ToggleSeparateSmtpCredentials)
                    .size(RADIO_SIZE),
                text("Use different credentials for SMTP")
                    .size(TEXT_LG)
                    .style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        );

        if self.auth_state.use_separate_smtp_credentials {
            col = col.push(labeled_input(
                "SMTP Username",
                "",
                &self.auth_state.smtp_username,
                AddAccountMessage::SmtpUsernameChanged,
            ));
            col = col.push(labeled_input(
                "SMTP Password",
                "",
                &self.auth_state.smtp_password,
                AddAccountMessage::SmtpPasswordChanged,
            ));
        }

        col = col.push(
            row![
                iced::widget::checkbox(self.auth_state.accept_invalid_certs)
                    .on_toggle(AddAccountMessage::ToggleAcceptInvalidCerts)
                    .size(RADIO_SIZE),
                text("Accept self-signed certificates")
                    .size(TEXT_LG)
                    .style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        );

        col.into()
    }
}

pub(super) fn view_validating<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Validating credentials...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        widgets::spinner(24.0),
        Space::new().height(SPACE_SM),
        text("Connecting to your mail server...")
            .size(TEXT_SM)
            .style(text::secondary),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

/// Test IMAP connection to validate credentials.
async fn validate_imap_connection(
    host: &str,
    port_str: &str,
    security: SecurityOption,
    username: &str,
    password: &str,
    accept_invalid_certs: bool,
) -> Result<(), String> {
    let port: u16 = port_str
        .parse()
        .map_err(|_| "Invalid port number".to_string())?;

    let security_str = security.to_db_string().to_string();

    rtsk::account::verify_imap::verify_imap_credentials(
        host,
        port,
        &security_str,
        username,
        password,
        "password",
        accept_invalid_certs,
    )
    .await
}
