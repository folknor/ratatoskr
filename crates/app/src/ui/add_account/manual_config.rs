use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Task};

use crate::font;
use crate::ui::layout::*;
use crate::ui::theme;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard, ManualAuthMethod,
    ManualProvider,
};
use super::views::{
    auth_method_selector, ghost_button, primary_button, security_selector, server_port_row,
};

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
                match self.manual_config.auth_method {
                    ManualAuthMethod::OAuth => {
                        self.resolved_auth_method = "oauth2".to_string();
                        self.step = AddAccountStep::OAuthWaiting;
                        self.error = Some(
                            "JMAP OAuth is not yet supported for manual configuration. \
                             Please use password authentication."
                                .to_string(),
                        );
                        self.step = AddAccountStep::ManualConfiguration;
                        (Task::none(), None)
                    }
                    ManualAuthMethod::Password => {
                        self.resolved_auth_method = "password".to_string();
                        self.step = AddAccountStep::PasswordAuth;
                        (Task::none(), None)
                    }
                }
            }
            ManualProvider::Imap => {
                self.resolved_auth_method = "password".to_string();
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }
        }
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
