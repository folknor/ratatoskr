use std::collections::{HashMap, HashSet};

use iced::widget::{Space, button, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use types::{FolderId, SidebarSelection, TagId};

use super::search_here::build_search_here_prefix;
use super::{Sidebar, SidebarMessage};

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
    let account_tags: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountTag))
        .collect();

    let has_hierarchy = account_labels.iter().any(|f| f.parent_id.is_some());

    let mut children: Vec<Element<'_, SidebarMessage>> = if has_hierarchy {
        render_label_tree(sidebar, &account_labels)
    } else {
        render_flat_labels(sidebar, &account_labels)
    };

    children.extend(render_tag_labels(sidebar, &account_tags));

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}

// ── Tree rendering helpers ───────────────────────────────

/// A node in the depth-first tree traversal.
struct TreeNode<'a> {
    folder: &'a NavigationFolder,
    depth: u16,
}

/// Maximum nesting depth to prevent cycles from crashing the renderer.
const MAX_TREE_DEPTH: u16 = 10;

/// Indent step per tree depth level.
const TREE_INDENT: f32 = SPACE_MD;

/// Sort folders into depth-first tree order and compute indent depth.
///
/// Roots (`parent_id == None`) come first, then their children recursively.
/// Items whose `parent_id` references a non-existent folder are treated as
/// roots (depth 0). A max-depth cap prevents infinite recursion from cycles.
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

    // Orphan recovery: items with parent_id pointing to a non-existent folder
    // won't appear in the tree. Add them as roots.
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

/// Check whether a folder is hidden because any of its ancestors is collapsed.
///
/// `id_to_folder` must be pre-built from the full label list to avoid
/// O(n^2) rebuilds when called per tree node.
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

/// Render labels as an indented tree with expand/collapse chevrons.
fn render_label_tree<'a>(
    sidebar: &'a Sidebar,
    labels: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    let tree = tree_sort(labels);
    let mut elements = Vec::new();

    // Build lookup map once for O(n) total instead of O(n^2)
    let id_to_folder: HashMap<&str, &NavigationFolder> =
        labels.iter().map(|f| (f.id.as_str(), *f)).collect();

    for node in &tree {
        // Skip if any ancestor is collapsed
        if is_hidden_by_collapsed_ancestor(node.folder, &id_to_folder, &sidebar.collapsed_folders) {
            continue;
        }

        let has_children = labels
            .iter()
            .any(|f| f.parent_id.as_deref() == Some(&node.folder.id));
        let is_collapsed = sidebar.collapsed_folders.contains(&node.folder.id);
        let active = sidebar.active_pinned_search.is_none()
            && matches!(
                &sidebar.selection,
                SidebarSelection::ProviderFolder(fid) if fid.as_str() == node.folder.id
            );
        let indent = TREE_INDENT * f32::from(node.depth);

        let mut item_row = row![].spacing(SPACE_XXS).align_y(Alignment::Center);

        // Indent spacer
        if indent > 0.0 {
            item_row = item_row.push(Space::new().width(indent));
        }

        // Expand/collapse chevron (only for parent folders)
        if has_children {
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
                .on_press(SidebarMessage::ToggleFolderExpand(node.folder.id.clone()))
                .padding(SPACE_XXXS)
                .style(theme::ButtonClass::Ghost.style()),
            );
        }

        // Color dot + label name
        item_row = item_row.push(widgets::color_dot(theme::avatar_color(&node.folder.name)));

        let lbl_style: fn(&iced::Theme) -> text::Style = if active {
            text::primary
        } else {
            text::secondary
        };
        item_row = item_row.push(text(&node.folder.name).size(TEXT_MD).style(lbl_style));

        // Unread badge
        if node.folder.unread_count > 0 {
            item_row = item_row
                .push(Space::new().width(Length::Fill))
                .push(widgets::count_badge(node.folder.unread_count));
        }

        let label_btn: Element<'_, SidebarMessage> = button(
            container(item_row)
                .padding(PAD_NAV_ITEM)
                .width(Length::Fill),
        )
        .on_press(SidebarMessage::Select(SidebarSelection::ProviderFolder(
            FolderId::from(node.folder.id.clone()),
        )))
        .padding(0)
        .style(theme::ButtonClass::Nav { active }.style())
        .width(Length::Fill)
        .into();

        let query_prefix = build_search_here_prefix(&node.folder.name, sidebar);
        elements.push(
            mouse_area(label_btn)
                .on_right_press(SidebarMessage::SearchHere(query_prefix))
                .into(),
        );
    }

    elements
}

/// Render labels as a flat list (Gmail / no hierarchy).
fn render_flat_labels<'a>(
    sidebar: &'a Sidebar,
    labels: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    labels
        .iter()
        .take(12)
        .map(|f| {
            let active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::ProviderFolder(fid) if fid.as_str() == f.id
                );
            let label_btn = widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                active,
                SidebarMessage::Select(SidebarSelection::ProviderFolder(FolderId::from(
                    f.id.clone(),
                ))),
            );
            let query_prefix = build_search_here_prefix(&f.name, sidebar);
            mouse_area(label_btn)
                .on_right_press(SidebarMessage::SearchHere(query_prefix))
                .into()
        })
        .collect()
}

fn render_tag_labels<'a>(
    sidebar: &'a Sidebar,
    folders: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    folders
        .iter()
        .map(|f| {
            let is_active = sidebar.active_pinned_search.is_none()
                && matches!(
                    &sidebar.selection,
                    SidebarSelection::Tag(tid) if tid.as_str() == f.id
                );
            widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                is_active,
                SidebarMessage::Select(SidebarSelection::Tag(TagId::from(f.id.clone()))),
            )
        })
        .collect()
}
