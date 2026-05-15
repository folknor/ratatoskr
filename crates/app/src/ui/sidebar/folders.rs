use std::collections::{HashMap, HashSet};

use iced::widget::{Space, button, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::{FolderId, SidebarSelection};

use super::search_here::build_search_here_user_folder_prefix;
use super::{Sidebar, SidebarMessage};

/// A node in the depth-first tree traversal.
struct TreeNode<'a> {
    folder: &'a NavigationFolder,
    depth: u16,
}

/// Maximum nesting depth to prevent cycles from crashing the renderer.
const MAX_TREE_DEPTH: u16 = 10;

/// Indent step per tree depth level.
const TREE_INDENT: f32 = SPACE_MD;

/// Render the account's user-created folders inline below Inbox in the
/// universal section. Returns an empty Vec in All-Accounts scope (user
/// folders don't aggregate across accounts) or when the current scope
/// has no `AccountFolder` items. Tree-aware: when any folder has a
/// parent, renders an indented tree with collapse chevrons; otherwise a
/// flat list.
pub(super) fn render_account_folders(sidebar: &Sidebar) -> Vec<Element<'_, SidebarMessage>> {
    if sidebar.is_all_accounts() {
        return Vec::new();
    }

    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let account_folders: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountFolder))
        .collect();

    if account_folders.is_empty() {
        return Vec::new();
    }

    let has_hierarchy = account_folders.iter().any(|f| f.parent_id.is_some());
    if has_hierarchy {
        render_folder_tree(sidebar, &account_folders)
    } else {
        render_folder_flat(sidebar, &account_folders)
    }
}

/// Depth-first ordering for tree rendering. Orphans (parent_id pointing
/// at a non-existent folder) are appended as roots.
fn tree_sort<'a>(folders: &[&'a NavigationFolder]) -> Vec<TreeNode<'a>> {
    let mut children_of: HashMap<Option<&str>, Vec<&NavigationFolder>> = HashMap::new();
    for f in folders {
        children_of
            .entry(f.parent_id.as_deref())
            .or_default()
            .push(f);
    }

    let mut result = Vec::with_capacity(folders.len());

    fn walk<'a>(
        parent: Option<&str>,
        depth: u16,
        children_of: &HashMap<Option<&str>, Vec<&'a NavigationFolder>>,
        result: &mut Vec<TreeNode<'a>>,
    ) {
        if depth > MAX_TREE_DEPTH {
            return;
        }
        let Some(children) = children_of.get(&parent) else {
            return;
        };
        for child in children {
            result.push(TreeNode {
                folder: child,
                depth,
            });
            walk(Some(&child.id), depth + 1, children_of, result);
        }
    }

    walk(None, 0, &children_of, &mut result);

    let in_tree: HashSet<&str> = result.iter().map(|n| n.folder.id.as_str()).collect();
    for f in folders {
        if !in_tree.contains(f.id.as_str()) {
            result.push(TreeNode {
                folder: f,
                depth: 0,
            });
        }
    }

    result
}

fn is_hidden_by_collapsed_ancestor(
    folder: &NavigationFolder,
    id_to_folder: &HashMap<&str, &NavigationFolder>,
    collapsed: &HashSet<String>,
) -> bool {
    let mut current_parent = folder.parent_id.as_deref();
    let mut depth = 0u16;
    while let Some(pid) = current_parent {
        if collapsed.contains(pid) {
            return true;
        }
        if depth >= MAX_TREE_DEPTH {
            break;
        }
        depth += 1;
        current_parent = id_to_folder.get(pid).and_then(|f| f.parent_id.as_deref());
    }
    false
}

fn render_folder_tree<'a>(
    sidebar: &'a Sidebar,
    folders: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    let tree = tree_sort(folders);
    let mut elements = Vec::new();

    let id_to_folder: HashMap<&str, &NavigationFolder> =
        folders.iter().map(|f| (f.id.as_str(), *f)).collect();

    for node in &tree {
        if is_hidden_by_collapsed_ancestor(node.folder, &id_to_folder, &sidebar.collapsed_folders) {
            continue;
        }

        let has_children = folders
            .iter()
            .any(|f| f.parent_id.as_deref() == Some(&node.folder.id));
        let is_collapsed = sidebar.collapsed_folders.contains(&node.folder.id);
        let active = sidebar.active_pinned_search.is_none()
            && matches!(
                &sidebar.selection,
                SidebarSelection::ProviderFolder(fid) if fid.as_str() == node.folder.id
            );
        let indent = TREE_INDENT * f32::from(node.depth);

        elements.push(folder_row(
            sidebar,
            node.folder,
            indent,
            Some((has_children, is_collapsed)),
            active,
        ));
    }

    elements
}

fn render_folder_flat<'a>(
    sidebar: &'a Sidebar,
    folders: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    folders
        .iter()
        .map(|f| {
            let active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::ProviderFolder(fid) if fid.as_str() == f.id
                );
            folder_row(sidebar, f, 0.0, None, active)
        })
        .collect()
}

/// One folder row. Matches the visual weight of `widgets::nav_button` in
/// the surrounding universal section: no color dot, no leading icon -
/// just an optional indent / chevron, the folder name, and the unread
/// badge. Right-click opens search-here.
fn folder_row<'a>(
    sidebar: &'a Sidebar,
    folder: &'a NavigationFolder,
    indent: f32,
    tree_state: Option<(bool, bool)>,
    active: bool,
) -> Element<'a, SidebarMessage> {
    let mut item_row = row![].spacing(SPACE_XXS).align_y(Alignment::Center);

    if indent > 0.0 {
        item_row = item_row.push(Space::new().width(indent));
    }

    if let Some((has_children, is_collapsed)) = tree_state
        && has_children
    {
        let chevron = if is_collapsed {
            icon::chevron_right()
        } else {
            icon::chevron_down()
        };
        item_row = item_row.push(
            button(
                chevron
                    .size(ICON_XS)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .on_press(SidebarMessage::ToggleFolderExpand(folder.id.clone()))
            .padding(SPACE_XXXS)
            .style(theme::ButtonClass::Ghost.style()),
        );
    }

    let lbl_style: fn(&iced::Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::TextClass::Muted.style()
    };
    item_row = item_row.push(text(&folder.name).size(TEXT_MD).style(lbl_style));

    if folder.unread_count > 0 {
        item_row = item_row
            .push(Space::new().width(Length::Fill))
            .push(widgets::count_badge(folder.unread_count));
    }

    let label_btn: Element<'_, SidebarMessage> = button(
        container(item_row)
            .padding(PAD_NAV_ITEM)
            .width(Length::Fill),
    )
    .on_press(SidebarMessage::Select(SidebarSelection::ProviderFolder(
        FolderId::from(folder.id.clone()),
    )))
    .padding(0)
    .style(theme::ButtonClass::Nav { active }.style())
    .width(Length::Fill)
    .into();

    let query_prefix = build_search_here_user_folder_prefix(&folder.name, sidebar);
    mouse_area(label_btn)
        .on_right_press(SidebarMessage::SearchHere(query_prefix))
        .into()
}
