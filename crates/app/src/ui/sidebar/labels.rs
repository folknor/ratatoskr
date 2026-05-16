use iced::widget::mouse_area;
use iced::Element;

use crate::ui::label_paint::LabelPaint;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::{LabelGroupId, SidebarSelection};

use super::search_here::build_search_here_prefix;
use super::{Sidebar, SidebarMessage};

/// LABELS section: explicit user-created label groups.
pub(super) fn labels_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let label_groups: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::LabelGroup))
        .collect();

    let children: Vec<Element<'_, SidebarMessage>> = label_groups
        .into_iter()
        .filter_map(|folder| {
            let group_id = folder.id.parse::<i64>().ok()?;
            Some((folder, LabelGroupId::from(group_id)))
        })
        .map(|(folder, group_id)| {
            let is_active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::LabelGroup(active) if *active == group_id
                );
            let paint = label_colors::LabelStyleHex::from_optional_pair(
                folder.color_bg.as_deref(),
                folder.color_fg.as_deref(),
            )
            .ok()
            .flatten()
            .map(LabelPaint::from_hex)
            .unwrap_or_else(|| LabelPaint::hashed_from_name(&folder.name));
            let item = widgets::label_nav_item(
                &folder.name,
                &folder.id,
                paint,
                is_active,
                folder.unread_count,
                SidebarMessage::Select(SidebarSelection::LabelGroup(group_id)),
            );
            let query_prefix = build_search_here_prefix(&folder.name, sidebar);
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
