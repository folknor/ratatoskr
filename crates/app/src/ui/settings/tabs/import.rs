use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;

pub(super) fn import_wizard_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref wizard) = state.import_wizard else {
        return column![].into();
    };

    let content: Element<'_, SettingsMessage> = match wizard.step {
        ImportStep::FileSelect => import_step_file_select(wizard),
        ImportStep::Mapping => import_step_mapping(wizard, &state.managed_accounts),
        ImportStep::VcfPreview => import_step_vcf_preview(wizard, &state.managed_accounts),
        ImportStep::Importing => import_step_importing(),
        ImportStep::Summary => import_step_summary(wizard),
    };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);
    col = col.push(
        text("Import Contacts")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );
    col = col.push(content);
    col.into()
}

fn import_step_file_select(_wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let items = vec![import_file_select_row()];
    section("Select File", items)
}

fn import_file_select_row() -> RowBuilder<'static> {
    let description = "Select a .csv, .xlsx, or .vcf file to import contacts from.";
    settings_row_container(
        SETTINGS_TOGGLE_ROW_HEIGHT,
        column![
            text("Choose a CSV, Excel, or vCard file to import.")
                .size(TEXT_LG)
                .style(text::base),
            Space::new().height(SPACE_XS),
            text(description)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            Space::new().height(SPACE_SM),
            text("Use the file browser to select a file. Supported formats: .csv, .xlsx, .vcf")
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        ],
    )
}

fn import_step_mapping<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    if let Some(ref path) = wizard.file_path {
        col = col.push(import_file_info_row(path));
    }

    col = col.push(import_header_toggle(wizard.has_header));

    if let Some(import::ImportPreview::Table(preview)) = wizard.preview.as_ref() {
        col = col.push(import_mapping_table(preview, &wizard.mappings));
        col = col.push(import_preview_stats(preview, &wizard.mappings));
    }

    col = col.push(import_account_selector(wizard, accounts));

    col = col.push(import_update_toggle(wizard.update_existing));

    col = col.push(import_execute_button());

    col.into()
}

fn import_file_info_row(path: &str) -> Element<'_, SettingsMessage> {
    section_untitled(vec![settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            icon::file().size(ICON_XL).style(text::secondary),
            Space::new().width(SPACE_XS),
            text(path).size(TEXT_LG).style(text::base),
        ]
        .align_y(Alignment::Center),
    )])
}

fn import_header_toggle(has_header: bool) -> Element<'static, SettingsMessage> {
    build_row(toggle_row(
        "First row is a header",
        "Enable if the first row contains column names, not data.",
        has_header,
        SettingsMessage::ImportToggleHeader,
    ))
}

fn import_mapping_table<'a>(
    preview: &'a ::import::TablePreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let mut items: Vec<RowBuilder<'a>> = Vec::new();

    for (i, header) in preview.headers.iter().enumerate() {
        let current_field = mappings
            .get(i)
            .copied()
            .unwrap_or(ImportContactField::Ignore);
        items.push(import_column_mapping_row(i, header, current_field));
    }

    let sample_count = preview.rows.len().min(5);
    if sample_count > 0 {
        items.push(import_sample_header());
        for row in preview.rows.iter().take(sample_count) {
            items.push(import_sample_row(row));
        }
    }

    section("Column Mapping", items)
}

fn import_column_mapping_row(
    index: usize,
    header: &str,
    current: ImportContactField,
) -> RowBuilder<'_> {
    let header_owned = header.to_string();
    let selected = current.label().to_string();

    Box::new(move |position| {
        button(
            container(
                row![
                    container(text(header_owned).size(TEXT_LG).style(text::base))
                        .align_y(Alignment::Center)
                        .width(Length::FillPortion(1)),
                    container(
                        text(format!("-> {selected}"))
                            .size(TEXT_LG)
                            .style(text::primary),
                    )
                    .align_y(Alignment::Center)
                    .width(Length::FillPortion(1)),
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::ImportMappingChanged(
            index,
            cycle_import_field(current),
        ))
        .padding(0)
        .style(move |theme, status| {
            theme::style_settings_row_button(theme, status, position)
        })
        .width(Length::Fill)
        .into()
    })
}

fn cycle_import_field(current: ImportContactField) -> ImportContactField {
    let all = ImportContactField::ALL_OPTIONS;
    let current_idx = all.iter().position(|&f| f == current).unwrap_or(0);
    let next_idx = (current_idx + 1) % all.len();
    all[next_idx]
}

fn import_sample_header<'a>() -> RowBuilder<'a> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text("Preview (first rows)")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    )
}

fn import_sample_row(row: &::import::ImportPreviewRow) -> RowBuilder<'_> {
    let display = row.cells.join("  |  ");
    let status = row.status;
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(display)
                .size(TEXT_SM)
                .style(if status.is_importable() { text::secondary } else { text::danger })
                .width(Length::Fill),
            text(status.label())
                .size(TEXT_SM)
                .style(if status.is_importable() { text::secondary } else { text::danger }),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
}

fn import_preview_stats<'a>(
    preview: &'a ::import::TablePreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let has_email = mappings.contains(&ImportContactField::Email);
    let status = if has_email {
        let skipped = preview.stats.skipped_total();
        if skipped > 0 {
            format!(
                "{} rows to import. {skipped} skipped.",
                preview.stats.importable
            )
        } else {
            format!("{} rows to import.", preview.stats.importable)
        }
    } else {
        "No Email column mapped. Map at least one column to Email.".to_string()
    };
    build_row(settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(status)
            .size(TEXT_LG)
            .style(if has_email { text::base } else { text::danger }),
    ))
}

fn import_account_selector<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let selected_id = wizard.account_id.as_deref();

    let mut btn_row = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    let is_local = selected_id.is_none();
    let local_style = if is_local {
        theme::ButtonClass::Primary
    } else {
        theme::ButtonClass::Ghost
    };
    btn_row = btn_row.push(
        button(text("Local").size(TEXT_SM))
            .style(local_style.style())
            .on_press(SettingsMessage::ImportAccountChanged(None))
            .padding(PAD_ICON_BTN),
    );

    for account in accounts {
        let is_selected = selected_id == Some(account.id.as_str());
        let style = if is_selected {
            theme::ButtonClass::Primary
        } else {
            theme::ButtonClass::Ghost
        };
        let aid = Some(account.id.clone());
        btn_row = btn_row.push(
            button(text(&account.email).size(TEXT_SM))
                .style(style.style())
                .on_press(SettingsMessage::ImportAccountChanged(aid))
                .padding(PAD_ICON_BTN),
        );
    }

    container(column![
        text("Import to account")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        Space::new().height(SPACE_XXXS),
        btn_row,
    ])
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn import_update_toggle(update_existing: bool) -> Element<'static, SettingsMessage> {
    build_row(toggle_row(
        "Update existing contacts",
        "When a duplicate email is found, update the existing contact with imported data.",
        update_existing,
        SettingsMessage::ImportToggleUpdateExisting,
    ))
}

fn import_execute_button() -> Element<'static, SettingsMessage> {
    container(
        button(container(text("Import").size(TEXT_LG)).center_x(Length::Fill))
            .on_press(SettingsMessage::ImportExecute)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fixed(EDITOR_BUTTON_WIDTH)),
    )
    .width(Length::Fill)
    .align_x(Alignment::End)
    .padding(PAD_SETTINGS_ROW)
    .into()
}

fn import_step_vcf_preview<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    if let Some(ref path) = wizard.file_path {
        col = col.push(import_file_info_row(path));
    }

    let Some(import::ImportPreview::Contacts(preview)) = wizard.preview.as_ref() else {
        return col.into();
    };

    let valid_count = preview.stats.importable;
    let total = preview.total_rows;
    let skipped = preview.stats.skipped_total();

    let stat_text =
        format!("{total} contacts found. {valid_count} with valid email, {skipped} without.",);
    col = col.push(build_row(settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(stat_text).size(TEXT_LG).style(text::base),
    )));

    let mut preview_items: Vec<RowBuilder<'a>> = Vec::new();
    for row in preview.rows.iter().take(10) {
        preview_items.push(import_vcf_contact_row(row));
    }
    if !preview_items.is_empty() {
        col = col.push(section("Preview", preview_items));
    }

    col = col.push(import_account_selector(wizard, accounts));

    col = col.push(import_update_toggle(wizard.update_existing));

    col = col.push(import_execute_button());

    col.into()
}

fn import_vcf_contact_row(row: &::import::ContactPreviewRow) -> RowBuilder<'_> {
    let contact = &row.contact;
    let name = contact
        .effective_display_name()
        .unwrap_or_else(|| "(no name)".to_string());
    let email = contact
        .normalized_email()
        .unwrap_or_else(|| "(no email)".to_string());

    let email_style: fn(&iced::Theme) -> text::Style = if row.status.is_importable() {
        text::secondary
    } else {
        text::danger
    };

    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(name)
                .size(TEXT_LG)
                .style(text::base)
                .width(Length::FillPortion(1)),
            text(email)
                .size(TEXT_SM)
                .style(email_style)
                .width(Length::FillPortion(1)),
            text(row.status.label())
                .size(TEXT_SM)
                .style(email_style)
                .width(Length::Shrink),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
}

fn import_step_importing() -> Element<'static, SettingsMessage> {
    build_row(settings_row_container(
        SETTINGS_TOGGLE_ROW_HEIGHT,
        column![
            text("Importing contacts...")
                .size(TEXT_LG)
                .style(text::base),
            Space::new().height(SPACE_XS),
            text("Please wait.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ],
    ))
}

fn import_step_summary(wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    if let Some(ref result) = wizard.result {
        let mut stats: Vec<RowBuilder<'_>> = Vec::new();
        stats.push(import_stat_row("Imported", result.imported));
        if result.updated > 0 {
            stats.push(import_stat_row("Updated", result.updated));
        }
        if result.skipped_no_email > 0 {
            stats.push(import_stat_row(
                "Skipped (no email)",
                result.skipped_no_email,
            ));
        }
        if result.skipped_invalid_email > 0 {
            stats.push(import_stat_row(
                "Skipped (invalid email)",
                result.skipped_invalid_email,
            ));
        }
        if result.skipped_duplicate > 0 {
            stats.push(import_stat_row(
                "Skipped (duplicate)",
                result.skipped_duplicate,
            ));
        }
        if result.groups_created > 0 {
            stats.push(import_stat_row("Groups created", result.groups_created));
        }
        col = col.push(section("Import Complete", stats));
    }

    col = col.push(
        container(
            button(container(text("Done").size(TEXT_LG)).center_x(Length::Fill))
                .on_press(SettingsMessage::ImportBack)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Primary.style())
                .width(Length::Fixed(EDITOR_BUTTON_WIDTH)),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(PAD_SETTINGS_ROW),
    );

    col.into()
}

fn import_stat_row(display_text: &str, count: usize) -> RowBuilder<'_> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(display_text)
                .size(TEXT_LG)
                .style(text::base)
                .width(Length::Fill),
            text(count.to_string()).size(TEXT_LG).style(text::primary),
        ]
        .align_y(Alignment::Center),
    )
}
