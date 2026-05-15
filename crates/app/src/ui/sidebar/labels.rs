use iced::widget::mouse_area;
use iced::Element;

use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::{SidebarSelection, LabelId};

use super::search_here::build_search_here_prefix;
use super::{Sidebar, SidebarMessage};

/// LABELS section: account-scoped labels only (Gmail user labels,
/// Exchange categories, IMAP/JMAP keywords). Folders moved to the
/// universal section - see `folders::render_account_folders`.
pub(super) fn labels_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let account_labels: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountLabel))
        .collect();

    let children: Vec<Element<'_, SidebarMessage>> = account_labels
        .iter()
        .map(|f| {
            let is_active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::Label(lid) if lid.as_str() == f.id
                );
            let dot_color = f
                .color_bg
                .as_deref()
                .map(theme::hex_to_color)
                .unwrap_or_else(|| theme::avatar_color(&f.name));
            let item = widgets::label_nav_item(
                &f.name,
                &f.id,
                dot_color,
                is_active,
                f.unread_count,
                SidebarMessage::Select(SidebarSelection::Label(LabelId::from(f.id.clone()))),
            );
            let query_prefix = build_search_here_prefix(&f.name, sidebar);
            mouse_area(item)
                .on_right_press(SidebarMessage::SearchHere(query_prefix))
                .into()
        })
        .collect();

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}
