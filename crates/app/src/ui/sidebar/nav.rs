use iced::widget::{column, mouse_area};
use iced::Element;

use crate::ui::layout::*;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::SidebarSelection;

use super::search_here::build_search_here_folder_prefix;
use super::{Sidebar, SidebarMessage, universal_folder_selection};

// ── Nav items ───────────────────────────────────────────

pub(super) fn nav_items(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let universal: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::Universal))
        .filter(|f| {
            // Spam and All Mail only when scoped to a single account
            if sidebar.is_all_accounts() {
                !matches!(f.id.as_str(), "SPAM" | "all-mail")
            } else {
                true
            }
        })
        .collect();

    let mut col = column![].spacing(SPACE_XXS);
    for f in &universal {
        let sel = universal_folder_selection(&f.id);
        let is_active = sidebar_nav_selection_is_active(sidebar, &sel);
        let nav_btn = widgets::nav_button(
            None,
            &f.name,
            is_active,
            widgets::NavSize::Compact,
            Some(f.unread_count),
            SidebarMessage::Select(sel),
        );
        let query_prefix = build_search_here_folder_prefix(&f.name, sidebar);
        col =
            col.push(mouse_area(nav_btn).on_right_press(SidebarMessage::SearchHere(query_prefix)));
    }
    col.into()
}

pub(super) fn sidebar_nav_selection_is_active(sidebar: &Sidebar, selection: &SidebarSelection) -> bool {
    sidebar.active_pinned_search.is_none() && sidebar.selection == *selection
}
