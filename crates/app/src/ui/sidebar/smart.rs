use iced::Element;

use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::FolderKind;
use types::SidebarSelection;

use super::{Sidebar, SidebarMessage};

pub(super) fn smart_folders(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let children: Vec<Element<'_, SidebarMessage>> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::SmartFolder))
        .map(|f| {
            let on_press = if let Some(ref query) = f.query {
                SidebarMessage::SelectSmartFolder {
                    id: f.id.clone(),
                    query: query.clone(),
                }
            } else {
                SidebarMessage::Select(SidebarSelection::SmartFolder { id: f.id.clone() })
            };
            let is_active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::SmartFolder { id } if id == &f.id
                );
            widgets::nav_button(
                None,
                &f.name,
                is_active,
                widgets::NavSize::Compact,
                Some(f.unread_count),
                on_press,
            )
        })
        .collect();

    widgets::collapsible_section(
        "SMART FOLDERS",
        sidebar.smart_folders_expanded,
        SidebarMessage::ToggleSmartFoldersSection,
        children,
    )
}
