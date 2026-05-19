use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Task};

use crate::font;
use crate::ui::layout::*;
use crate::ui::theme;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard, CustomOidcConfig,
    ManualAuthMethod, ManualProvider,
};
use super::views::{
    auth_method_selector, ghost_button, primary_button, security_selector, server_port_row,
};

/// Three labeled text inputs - issuer URL, client ID, client secret - for
/// the `CustomOidc*` provider variants. Client fields are optional;
/// dynamic client registration (RFC 7591) fills them in when the user
/// leaves them blank and the issuer advertises a registration endpoint.
fn custom_oidc_fields(cfg: &CustomOidcConfig) -> Element<'_, AddAccountMessage> {
    fn field<'a>(
        label: &'a str,
        placeholder: &'a str,
        value: &'a str,
        on_change: fn(String) -> AddAccountMessage,
    ) -> Element<'a, AddAccountMessage> {
        column![
            text(label).size(TEXT_SM).style(text::secondary),
            text_input(placeholder, value)
                .on_input(on_change)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        ]
        .spacing(SPACE_XXXS)
        .into()
    }

    column![
        text("OIDC Provider").size(TEXT_XL).style(text::base),
        field(
            "Issuer URL",
            "https://auth.corp.example",
            &cfg.issuer_url,
            AddAccountMessage::CustomOidcIssuerChanged,
        ),
        field(
            "Client ID (optional - leave blank to auto-register)",
            "ratatoskr-corp",
            &cfg.client_id,
            AddAccountMessage::CustomOidcClientIdChanged,
        ),
        field(
            "Client Secret (optional - for confidential clients)",
            "",
            &cfg.client_secret,
            AddAccountMessage::CustomOidcClientSecretChanged,
        ),
    ]
    .spacing(SPACE_SM)
    .into()
}

impl AddAccountWizard {
    pub(super) fn handle_submit_manual_config(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        let Some(provider) = self.manual_config.selected_provider else {
            self.error = Some("Please select a provider type.".to_string());
            return (Task::none(), None);
        };

        self.prefill_from_email();
        self.resolved_provider = provider.to_provider_string().to_string();
        self.error = None;

        match provider {
            ManualProvider::Gmail | ManualProvider::Microsoft365 => {
                // OAuth providers - look up OAuth config from the registry
                let provider_id = match provider {
                    ManualProvider::Gmail => "google",
                    ManualProvider::Microsoft365 => "microsoft",
                    _ => unreachable!(),
                };
                match self.manual_config.auth_method {
                    ManualAuthMethod::OAuth => {
                        self.resolved_auth_method = "oauth2".to_string();
                        let task = self.start_reauth_oauth(Some(provider_id), None, None);
                        (task, None)
                    }
                    ManualAuthMethod::Password => {
                        self.resolved_auth_method = "password".to_string();
                        self.step = AddAccountStep::PasswordAuth;
                        (Task::none(), None)
                    }
                }
            }
            ManualProvider::Jmap => {
                if self.manual_config.jmap_url.trim().is_empty() {
                    self.error = Some("Please enter a JMAP session URL.".to_string());
                    return (Task::none(), None);
                }
                // ManualProvider::Jmap is password-only now; the OAuth
                // path for JMAP lives on ManualProvider::CustomOidcJmap.
                // The auth_method toggle on this variant is a no-op for
                // OAuth selection (kept to avoid breaking message
                // plumbing); we always end up in PasswordAuth.
                self.resolved_auth_method = "password".to_string();
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }
            ManualProvider::Imap => {
                self.resolved_auth_method = "password".to_string();
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }
            ManualProvider::CustomOidcImap => self.submit_custom_oidc_imap(),
            ManualProvider::CustomOidcJmap => self.submit_custom_oidc_jmap(),
        }
    }

    fn submit_custom_oidc_imap(&mut self) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if let Err(msg) = self.validate_custom_oidc_issuer() {
            self.error = Some(msg);
            return (Task::none(), None);
        }
        if self.auth_state.imap_host.trim().is_empty() {
            self.error = Some("Please enter an IMAP host.".to_string());
            return (Task::none(), None);
        }
        if self.auth_state.smtp_host.trim().is_empty() {
            self.error = Some("Please enter an SMTP host.".to_string());
            return (Task::none(), None);
        }
        let issuer = self
            .manual_config
            .custom_oidc
            .issuer_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        self.resolved_provider = format!("oidc:{issuer}");
        self.resolved_auth_method = "oauth2".to_string();
        let client_id = self.manual_config.custom_oidc.client_id.clone();
        let task = self.start_reauth_oauth(Some(&self.resolved_provider.clone()), Some(&client_id), None);
        (task, None)
    }

    fn submit_custom_oidc_jmap(&mut self) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if let Err(msg) = self.validate_custom_oidc_issuer() {
            self.error = Some(msg);
            return (Task::none(), None);
        }
        if self.manual_config.jmap_url.trim().is_empty() {
            self.error = Some("Please enter a JMAP session URL.".to_string());
            return (Task::none(), None);
        }
        let issuer = self
            .manual_config
            .custom_oidc
            .issuer_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        self.resolved_provider = format!("oidc:{issuer}");
        self.resolved_auth_method = "oauth2".to_string();
        let client_id = self.manual_config.custom_oidc.client_id.clone();
        let task = self.start_reauth_oauth(Some(&self.resolved_provider.clone()), Some(&client_id), None);
        (task, None)
    }

    /// Validate the user-supplied issuer URL: non-empty, parseable, and
    /// HTTPS (or HTTP at the configured test base in harness mode).
    fn validate_custom_oidc_issuer(&self) -> Result<(), String> {
        let trimmed = self.manual_config.custom_oidc.issuer_url.trim();
        if trimmed.is_empty() {
            return Err("Please enter an OIDC issuer URL.".to_string());
        }
        if !rtsk::discovery::oidc::is_valid_https_url(trimmed) {
            return Err(
                "Issuer URL must be HTTPS, with no embedded credentials or fragment."
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn view_manual_config(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        col = col.push(
            text("Manual Configuration")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        // Provider type selection
        col = col.push(text("Provider").size(TEXT_SM).style(text::secondary));
        let mut provider_row = row![].spacing(SPACE_XS);
        for &p in ManualProvider::ALL {
            let selected = self.manual_config.selected_provider == Some(p);
            let style = if selected {
                theme::ButtonClass::ProtocolCardSelected
            } else {
                theme::ButtonClass::ProtocolCard
            };
            provider_row = provider_row.push(
                button(
                    container(text(p.label()).size(TEXT_LG).style(text::base))
                        .padding(PAD_CARD)
                        .center_x(Length::Fill),
                )
                .on_press(AddAccountMessage::SelectManualProvider(p))
                .padding(0)
                .style(style.style())
                .width(Length::FillPortion(1)),
            );
        }
        col = col.push(provider_row);

        // Show fields based on selected provider
        match self.manual_config.selected_provider {
            Some(ManualProvider::Imap) => {
                col = col.push(Space::new().height(SPACE_XS));
                col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
                col = col.push(server_port_row(
                    "imap.example.com",
                    &self.auth_state.imap_host,
                    "993",
                    &self.auth_state.imap_port,
                    AddAccountMessage::ManualImapHostChanged,
                    AddAccountMessage::ManualImapPortChanged,
                ));
                col = col.push(security_selector(
                    self.auth_state.imap_security,
                    AddAccountMessage::ManualImapSecurityChanged,
                ));

                col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
                col = col.push(server_port_row(
                    "smtp.example.com",
                    &self.auth_state.smtp_host,
                    "587",
                    &self.auth_state.smtp_port,
                    AddAccountMessage::ManualSmtpHostChanged,
                    AddAccountMessage::ManualSmtpPortChanged,
                ));
                col = col.push(security_selector(
                    self.auth_state.smtp_security,
                    AddAccountMessage::ManualSmtpSecurityChanged,
                ));
            }
            Some(ManualProvider::Jmap) => {
                col = col.push(Space::new().height(SPACE_XS));
                col = col.push(
                    column![
                        text("JMAP Session URL")
                            .size(TEXT_SM)
                            .style(text::secondary),
                        text_input(
                            "https://jmap.example.com/.well-known/jmap",
                            &self.manual_config.jmap_url
                        )
                        .on_input(AddAccountMessage::ManualJmapUrlChanged)
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::TextInputClass::Settings.style())
                        .width(Length::Fill),
                    ]
                    .spacing(SPACE_XXXS),
                );

                // Auth method selector
                col = col.push(text("Authentication").size(TEXT_SM).style(text::secondary));
                col = col.push(auth_method_selector(self.manual_config.auth_method));
            }
            Some(ManualProvider::CustomOidcImap) => {
                col = col.push(Space::new().height(SPACE_XS));
                col = col.push(custom_oidc_fields(&self.manual_config.custom_oidc));

                col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
                col = col.push(server_port_row(
                    "imap.corp.example",
                    &self.auth_state.imap_host,
                    "993",
                    &self.auth_state.imap_port,
                    AddAccountMessage::ManualImapHostChanged,
                    AddAccountMessage::ManualImapPortChanged,
                ));
                col = col.push(security_selector(
                    self.auth_state.imap_security,
                    AddAccountMessage::ManualImapSecurityChanged,
                ));

                col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
                col = col.push(server_port_row(
                    "smtp.corp.example",
                    &self.auth_state.smtp_host,
                    "587",
                    &self.auth_state.smtp_port,
                    AddAccountMessage::ManualSmtpHostChanged,
                    AddAccountMessage::ManualSmtpPortChanged,
                ));
                col = col.push(security_selector(
                    self.auth_state.smtp_security,
                    AddAccountMessage::ManualSmtpSecurityChanged,
                ));
            }
            Some(ManualProvider::CustomOidcJmap) => {
                col = col.push(Space::new().height(SPACE_XS));
                col = col.push(custom_oidc_fields(&self.manual_config.custom_oidc));

                col = col.push(
                    column![
                        text("JMAP Session URL")
                            .size(TEXT_SM)
                            .style(text::secondary),
                        text_input(
                            "https://jmap.corp.example/.well-known/jmap",
                            &self.manual_config.jmap_url
                        )
                        .on_input(AddAccountMessage::ManualJmapUrlChanged)
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::TextInputClass::Settings.style())
                        .width(Length::Fill),
                    ]
                    .spacing(SPACE_XXXS),
                );
            }
            Some(ManualProvider::Gmail) | Some(ManualProvider::Microsoft365) => {
                col = col.push(Space::new().height(SPACE_XS));
                // Auth method selector
                col = col.push(text("Authentication").size(TEXT_SM).style(text::secondary));
                col = col.push(auth_method_selector(self.manual_config.auth_method));

                // If password auth, show IMAP/SMTP fields
                if self.manual_config.auth_method == ManualAuthMethod::Password {
                    col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
                    col = col.push(server_port_row(
                        "imap.example.com",
                        &self.auth_state.imap_host,
                        "993",
                        &self.auth_state.imap_port,
                        AddAccountMessage::ManualImapHostChanged,
                        AddAccountMessage::ManualImapPortChanged,
                    ));
                    col = col.push(security_selector(
                        self.auth_state.imap_security,
                        AddAccountMessage::ManualImapSecurityChanged,
                    ));

                    col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
                    col = col.push(server_port_row(
                        "smtp.example.com",
                        &self.auth_state.smtp_host,
                        "587",
                        &self.auth_state.smtp_port,
                        AddAccountMessage::ManualSmtpHostChanged,
                        AddAccountMessage::ManualSmtpPortChanged,
                    ));
                    col = col.push(security_selector(
                        self.auth_state.smtp_security,
                        AddAccountMessage::ManualSmtpSecurityChanged,
                    ));
                }
            }
            None => {
                col = col.push(
                    text("Select a provider type above to continue.")
                        .size(TEXT_SM)
                        .style(text::secondary),
                );
            }
        }

        col = col.push(Space::new().height(SPACE_SM));
        if self.manual_config.selected_provider.is_some() {
            col = col.push(primary_button(
                "Continue",
                AddAccountMessage::SubmitManualConfig,
            ));
        }
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        scrollable(col).spacing(SCROLLBAR_SPACING).into()
    }
}
