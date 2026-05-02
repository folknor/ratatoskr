use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use rtsk::scope::ViewScope;

use super::{Sidebar, SidebarMessage};

// ── Pinned public folders ────────────────────────────────

pub(super) fn pinned_public_folders_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let items: Vec<Element<'_, SidebarMessage>> = sidebar
        .pinned_public_folders
        .iter()
        .map(|pf| {
            let active = matches!(
                &sidebar.selected_scope,
                ViewScope::PublicFolder { folder_id, .. } if *folder_id == pf.folder_id
            );
            let label = &pf.display_name;
            let count = pf.unread_count;

            let mut row_content = row![
                icon::folder().size(ICON_SM).style(text::secondary),
                text(label)
                    .size(TEXT_SM)
                    .style(if active { text::primary } else { text::base }),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .width(Length::Fill);

            if count > 0 {
                row_content = row_content.push(
                    text(format!("{count}"))
                        .size(TEXT_XS)
                        .style(text::secondary),
                );
            }

            let style = if active {
                theme::ButtonClass::Nav { active: true }.style()
            } else {
                theme::ButtonClass::Nav { active: false }.style()
            };

            button(
                container(row_content)
                    .padding(PAD_NAV_ITEM)
                    .width(Length::Fill),
            )
            .on_press(SidebarMessage::SelectPublicFolder(
                pf.account_id.clone(),
                pf.folder_id.clone(),
            ))
            .padding(0)
            .width(Length::Fill)
            .style(style)
            .into()
        })
        .collect();

    let header = text("PUBLIC FOLDERS").size(TEXT_XS).style(text::secondary);

    let mut col = column![header].spacing(SPACE_XXXS);
    for item in items {
        col = col.push(item);
    }
    col.into()
}
