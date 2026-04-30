use iced::widget::column;
use iced::{Element, Length};

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::widgets;

pub(super) fn ai_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Provider",
        vec![setting_row(
            "AI Provider",
            widgets::select(
                &["Claude", "OpenAI", "Gemini", "Ollama", "Copilot"],
                &state.ai_provider,
                state.open_select == Some(SelectField::AiProvider),
                SettingsMessage::ToggleSelect(SelectField::AiProvider),
                SettingsMessage::AiProviderChanged,
            ),
            SettingsMessage::ToggleSelect(SelectField::AiProvider),
        )],
    ));

    if state.ai_provider == "Ollama" {
        col = col.push(section(
            "Local Server",
            vec![
                input_row(
                    "ollama-url",
                    "Server URL",
                    "http://localhost:11434",
                    state.ai_ollama_url.text(),
                    SettingsMessage::OllamaUrlChanged,
                    InputField::OllamaUrl,
                ),
                input_row(
                    "ollama-model",
                    "Model Name",
                    "e.g. llama3.2",
                    state.ai_ollama_model.text(),
                    SettingsMessage::OllamaModelChanged,
                    InputField::OllamaModel,
                ),
            ],
        ));
    } else {
        let key_label = match state.ai_provider.as_str() {
            "OpenAI" => "OpenAI API Key",
            "Gemini" => "Google AI API Key",
            "Copilot" => "GitHub Personal Access Token",
            _ => "Anthropic API Key",
        };

        let model_options: &[&str] = match state.ai_provider.as_str() {
            "OpenAI" => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
            "Gemini" => &[
                "gemini-2.0-flash",
                "gemini-2.5-flash-preview-05-20",
                "gemini-2.5-pro",
            ],
            "Copilot" => &["openai/gpt-4o", "openai/gpt-4o-mini"],
            _ => &[
                "claude-haiku-4-5-20251001",
                "claude-sonnet-4-5",
                "claude-sonnet-4-6",
                "claude-opus-4-6",
            ],
        };

        col = col.push(section(
            "API Key",
            vec![
                input_row_secure(
                    "ai-api-key",
                    key_label,
                    "",
                    state.ai_api_key.text(),
                    SettingsMessage::AiApiKeyChanged,
                    InputField::AiApiKey,
                ),
                setting_row(
                    "Model",
                    widgets::select(
                        model_options,
                        &state.ai_model,
                        state.open_select == Some(SelectField::AiModel),
                        SettingsMessage::ToggleSelect(SelectField::AiModel),
                        SettingsMessage::AiModelChanged,
                    ),
                    SettingsMessage::ToggleSelect(SelectField::AiModel),
                ),
            ],
        ));
    }

    col = col.push(section_with_subtitle(
        "Features",
        "AI-powered tools to help manage your inbox",
        vec![
            toggle_row(
                "Enable AI Features",
                "Use AI-powered features across the app",
                state.ai_enabled,
                SettingsMessage::ToggleAiEnabled,
            ),
            toggle_row(
                "Auto-Categorize",
                "Automatically categorize incoming emails",
                state.ai_auto_categorize,
                SettingsMessage::ToggleAiAutoCategorize,
            ),
            toggle_row(
                "Auto-Summarize",
                "Generate summaries for long email threads",
                state.ai_auto_summarize,
                SettingsMessage::ToggleAiAutoSummarize,
            ),
        ],
    ));

    col = col.push(section(
        "Auto-Draft Replies",
        vec![
            toggle_row(
                "Auto-Draft",
                "Automatically draft replies based on email content",
                state.ai_auto_draft,
                SettingsMessage::ToggleAiAutoDraft,
            ),
            toggle_row(
                "Learn Writing Style",
                "Analyze your sent emails to match your writing style",
                state.ai_writing_style,
                SettingsMessage::ToggleAiWritingStyle,
            ),
        ],
    ));

    col = col.push(section(
        "Auto-Archive Categories",
        vec![
            toggle_row(
                "Updates",
                "Automatically archive update emails",
                state.ai_auto_archive_updates,
                SettingsMessage::ToggleAiAutoArchiveUpdates,
            ),
            toggle_row(
                "Promotions",
                "Automatically archive promotional emails",
                state.ai_auto_archive_promotions,
                SettingsMessage::ToggleAiAutoArchivePromotions,
            ),
            toggle_row(
                "Social",
                "Automatically archive social notification emails",
                state.ai_auto_archive_social,
                SettingsMessage::ToggleAiAutoArchiveSocial,
            ),
            toggle_row(
                "Newsletters",
                "Automatically archive newsletters",
                state.ai_auto_archive_newsletters,
                SettingsMessage::ToggleAiAutoArchiveNewsletters,
            ),
        ],
    ));

    col.into()
}
