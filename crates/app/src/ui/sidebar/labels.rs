use iced::widget::mouse_area;
use iced::Element;

use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::{SidebarSelection, TagId};

use super::search_here::build_search_here_prefix;
use super::{Sidebar, SidebarMessage};

/// LABELS section: account-scoped tags only (Gmail user labels,
/// Exchange categories, IMAP/JMAP keywords). Folders moved to the
/// universal section - see `folders::render_account_folders`.
pub(super) fn labels_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let account_tags: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountTag))
        .collect();

    let children: Vec<Element<'_, SidebarMessage>> = account_tags
        .iter()
        .map(|f| {
            let is_active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::Tag(tid) if tid.as_str() == f.id
                );
            let item = widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                is_active,
                SidebarMessage::Select(SidebarSelection::Tag(TagId::from(f.id.clone()))),
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
