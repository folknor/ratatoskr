#![allow(dead_code)]

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::db::ThreadAttachment;
use crate::icon;
use crate::ui::layout::{
    ATTACHMENT_ACTION_BTN_HEIGHT, ATTACHMENT_CARD_MAIN_ROW_HEIGHT, ATTACHMENT_CARD_META_ROW_HEIGHT,
    ATTACHMENT_ICON_BTN_WIDTH, ICON_ATTACHMENT_FILE, ICON_MD, ICON_XS, PAD_ICON_BTN, PAD_NAV_ITEM,
    SPACE_MD, SPACE_XS, SPACE_XXS, SPACE_XXXS, TEXT_MD, TEXT_SM, TEXT_XS,
};
use crate::ui::theme;

/// Click-target messages for an attachment card.
///
/// Bundled into a struct to keep `attachment_card`'s arity below the
/// `too_many_arguments` clippy threshold.
pub struct AttachmentCardActions<M> {
    /// Row-body click: open with the system default handler.
    pub on_open: M,
    /// Save icon click: save with file picker.
    pub on_save: M,
    /// "N versions" toggle. `None` when the card has only one version.
    pub on_toggle_versions: Option<M>,
    /// Meta line click: pop out the parent message of the latest version.
    pub on_pop_out_latest: M,
    /// One-per-version: pop out the parent message of each version. Index
    /// matches `versions[]`.
    pub on_pop_out_versions: Vec<M>,
}

pub fn attachment_card<'a, M: 'a + Clone>(
    primary: &'a ThreadAttachment,
    versions: &[&'a ThreadAttachment],
    expanded: bool,
    actions: AttachmentCardActions<M>,
) -> Element<'a, M> {
    let filename = primary.filename.as_deref().unwrap_or("(unnamed)");
    let file_icon = file_type_icon(primary.mime_type.as_deref());
    let latest = versions.first().copied().unwrap_or(primary);
    let date_sender = format_attachment_date_sender(latest);

    let top_row = row![
        container(
            text(filename)
                .size(TEXT_MD)
                .style(text::base)
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
        container(
            text(format_file_size(latest.size))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let meta_btn = button(
        text(date_sender)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .on_press(actions.on_pop_out_latest.clone())
    .style(theme::ButtonClass::Ghost.style())
    .padding(0);

    let mut bottom_row = row![
        container(meta_btn)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    if let Some(toggle_msg) = actions.on_toggle_versions {
        let chevron = if expanded {
            icon::chevron_down()
        } else {
            icon::chevron_right()
        };
        let versions_btn = button(
            row![
                chevron
                    .size(ICON_XS)
                    .style(theme::TextClass::Tertiary.style()),
                text(format!("{} versions", versions.len()))
                    .size(TEXT_XS)
                    .line_height(iced::widget::text::LineHeight::Relative(1.0))
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .on_press(toggle_msg)
        .style(theme::ButtonClass::Ghost.style())
        .padding(PAD_ICON_BTN);
        bottom_row = bottom_row.push(versions_btn);
    }

    let bottom_row_pinned = container(bottom_row)
        .height(Length::Fixed(ATTACHMENT_CARD_META_ROW_HEIGHT))
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let text_col = column![top_row, bottom_row_pinned]
        .spacing(SPACE_XXXS)
        .width(Length::Fill);

    let main_row = container(
        row![
            container(file_icon.size(ICON_ATTACHMENT_FILE).style(text::secondary),)
                .align_y(Alignment::Center),
            text_col,
        ]
        .spacing(SPACE_MD)
        .align_y(Alignment::Center),
    )
    .height(Length::Fixed(ATTACHMENT_CARD_MAIN_ROW_HEIGHT))
    .align_y(Alignment::Center);

    let mut card_col = column![main_row].spacing(SPACE_XXXS);

    if expanded && versions.len() > 1 {
        card_col = card_col.push(Space::new().height(SPACE_XXS));
        for (i, ver) in versions.iter().enumerate() {
            let label = format_attachment_version_line(ver, i == 0);
            let on_press = actions
                .on_pop_out_versions
                .get(i)
                .cloned()
                .unwrap_or_else(|| actions.on_pop_out_latest.clone());
            let version_btn = button(
                text(label)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .on_press(on_press)
            .style(theme::ButtonClass::Ghost.style())
            .padding(0)
            .width(Length::Fill);
            card_col = card_col.push(
                row![
                    Space::new().width(ICON_ATTACHMENT_FILE + SPACE_MD),
                    version_btn,
                ]
                .align_y(Alignment::Center),
            );
        }
        card_col = card_col.push(Space::new().height(SPACE_XS));
    }

    let info_card = container(card_col)
        .padding(PAD_NAV_ITEM)
        .style(theme::ContainerClass::EmailBody.style())
        .width(Length::Fill);

    let open_btn = button(
        container(icon::external_link().size(ICON_MD).style(text::secondary))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .height(Length::Fill)
            .width(Length::Fill),
    )
    .on_press(actions.on_open)
    .style(theme::ButtonClass::Ghost.style())
    .padding(0)
    .height(Length::Fixed(ATTACHMENT_ACTION_BTN_HEIGHT))
    .width(Length::Fixed(ATTACHMENT_ICON_BTN_WIDTH));

    let save_btn = button(
        container(icon::download().size(ICON_MD).style(text::secondary))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .height(Length::Fill)
            .width(Length::Fill),
    )
    .on_press(actions.on_save)
    .style(theme::ButtonClass::Ghost.style())
    .padding(0)
    .height(Length::Fixed(ATTACHMENT_ACTION_BTN_HEIGHT))
    .width(Length::Fixed(ATTACHMENT_ICON_BTN_WIDTH));

    row![info_card, open_btn, save_btn]
        .spacing(SPACE_XS)
        .align_y(Alignment::Start)
        .into()
}

fn file_type_icon<'a>(mime_type: Option<&str>) -> iced::widget::Text<'a> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}

fn format_attachment_meta(att: &ThreadAttachment) -> String {
    let type_label = mime_to_type_label(att.mime_type.as_deref());
    let size = format_file_size(att.size);
    let date = att
        .date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d").to_string())
        .unwrap_or_default();
    let sender = att.from_name.as_deref().unwrap_or("unknown");
    format!("{type_label} \u{00B7} {size} \u{00B7} {date} from {sender}")
}

/// "Mar 14 from Alex Morgan" - file size lives separately in the top-right
/// of the card now, so the meta line carries only the sender context.
fn format_attachment_date_sender(att: &ThreadAttachment) -> String {
    let date = att
        .date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d").to_string())
        .unwrap_or_default();
    let sender = att.from_name.as_deref().unwrap_or("unknown");
    format!("{date} from {sender}")
}

/// Per-version line shown under an expanded attachment card. Drops the type
/// label (already implied by the parent filename + icon) and tags the latest.
fn format_attachment_version_line(att: &ThreadAttachment, is_latest: bool) -> String {
    let size = format_file_size(att.size);
    let date = att
        .date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d").to_string())
        .unwrap_or_default();
    let sender = att.from_name.as_deref().unwrap_or("unknown");
    if is_latest {
        format!("{size} \u{00B7} {date} from {sender} (latest)")
    } else {
        format!("{size} \u{00B7} {date} from {sender}")
    }
}

fn mime_to_type_label(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        t if t.starts_with("image/") => "Image",
        t if t.contains("pdf") => "PDF",
        t if t.contains("spreadsheet") || t.contains("excel") => "Excel",
        t if t.contains("word") || t.contains("document") => "Word",
        t if t.contains("zip") || t.contains("archive") => "Archive",
        _ => "File",
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}
