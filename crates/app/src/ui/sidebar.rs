use std::collections::{HashMap, HashSet};

use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Task};

use crate::component::Component;
use crate::db::{Account, PinnedSearch};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets::{self, DropdownEntry, DropdownIcon};
use ratatoskr_core::db::queries_extra::navigation::{
    FolderKind, NavigationFolder, NavigationState,
};

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarMessage {
    SelectAccount(usize),
    SelectAllAccounts,
    CycleAccount,
    SelectLabel(Option<String>),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
    ToggleFolderExpand(String),
    Compose,
    ToggleSettings,
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
    RefreshPinnedSearch(i64),
    ToggleMode,
    /// Right-click "Search here" on a label/folder.
    SearchHere(String),
}

/// Events the sidebar emits upward to the App.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    AccountSelected(usize),
    AllAccountsSelected,
    /// Emitted when account cycling completes. The parent should reload
    /// navigation state — the sidebar already emits `AccountSelected`
    /// for the actual account change, so this is supplementary.
    CycleAccount,
    LabelSelected(Option<String>),
    Compose,
    ToggleSettings,
    PinnedSearchSelected(i64),
    PinnedSearchDismissed(i64),
    PinnedSearchRefreshed(i64),
    ModeToggled,
    /// "Search here" — prefill search bar with a scope query prefix.
    SearchHere { query_prefix: String },
}

// ── Sidebar layout constants ─────────────────────────────

/// Maximum display length for pinned search query text before truncation.
const PINNED_SEARCH_QUERY_MAX_CHARS: usize = 28;

// ── State ──────────────────────────────────────────────

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub nav_state: Option<NavigationState>,
    pub selected_account: Option<usize>,
    pub selected_label: Option<String>,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
    /// Set of folder IDs whose children are collapsed (hidden).
    pub collapsed_folders: HashSet<String>,
    /// Pinned searches, set by parent App before each view.
    pub pinned_searches: Vec<PinnedSearch>,
    /// Currently selected pinned search, set by parent App.
    pub active_pinned_search: Option<i64>,
    /// Whether the app is in calendar mode. Set by the parent App.
    pub in_calendar_mode: bool,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            nav_state: None,
            selected_account: None,
            selected_label: None,
            scope_dropdown_open: false,
            labels_expanded: true,
            smart_folders_expanded: true,
            collapsed_folders: HashSet::new(),
            pinned_searches: Vec::new(),
            active_pinned_search: None,
            in_calendar_mode: false,
        }
    }

    pub fn is_all_accounts(&self) -> bool {
        self.selected_account.is_none()
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for Sidebar {
    type Message = SidebarMessage;
    type Event = SidebarEvent;

    fn update(
        &mut self,
        message: SidebarMessage,
    ) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        match message {
            SidebarMessage::SelectAccount(idx) => {
                self.selected_account = Some(idx);
                self.selected_label = None;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AccountSelected(idx)))
            }
            SidebarMessage::SelectAllAccounts => {
                self.selected_account = None;
                self.selected_label = None;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AllAccountsSelected))
            }
            SidebarMessage::CycleAccount => {
                if self.accounts.len() > 1 {
                    let next = match self.selected_account {
                        Some(idx) => (idx + 1) % self.accounts.len(),
                        None => 0,
                    };
                    self.selected_account = Some(next);
                    self.selected_label = None;
                    self.scope_dropdown_open = false;
                    (Task::none(), Some(SidebarEvent::AccountSelected(next)))
                } else {
                    (Task::none(), None)
                }
            }
            SidebarMessage::SelectLabel(label_id) => {
                self.selected_label = label_id.clone();
                (Task::none(), Some(SidebarEvent::LabelSelected(label_id)))
            }
            SidebarMessage::ToggleScopeDropdown => {
                self.scope_dropdown_open = !self.scope_dropdown_open;
                (Task::none(), None)
            }
            SidebarMessage::ToggleLabelsSection => {
                self.labels_expanded = !self.labels_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleSmartFoldersSection => {
                self.smart_folders_expanded = !self.smart_folders_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleFolderExpand(folder_id) => {
                if !self.collapsed_folders.remove(&folder_id) {
                    self.collapsed_folders.insert(folder_id);
                }
                (Task::none(), None)
            }
            SidebarMessage::Compose => {
                (Task::none(), Some(SidebarEvent::Compose))
            }
            SidebarMessage::ToggleSettings => {
                (Task::none(), Some(SidebarEvent::ToggleSettings))
            }
            SidebarMessage::SelectPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchSelected(id)))
            }
            SidebarMessage::DismissPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchDismissed(id)))
            }
            SidebarMessage::RefreshPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchRefreshed(id)))
            }
            SidebarMessage::ToggleMode => {
                (Task::none(), Some(SidebarEvent::ModeToggled))
            }
            SidebarMessage::SearchHere(query_prefix) => {
                (Task::none(), Some(SidebarEvent::SearchHere { query_prefix }))
            }
        }
    }

    fn view(&self) -> Element<'_, SidebarMessage> {
        let show_labels = self.selected_account.is_some();

        // Mode toggle + scope dropdown header
        let mode_icon = if self.in_calendar_mode {
            icon::mail()
        } else {
            icon::calendar()
        };
        let mode_btn = button(
            container(mode_icon.size(ICON_SM))
                .center(Length::Shrink),
        )
        .on_press(SidebarMessage::ToggleMode)
        .padding(SPACE_XS)
        .style(theme::ButtonClass::Ghost.style());

        let header_row = row![mode_btn, scope_dropdown(self)]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center)
            .width(Length::Fill);

        let mut col = column![
            header_row,
            Space::new().height(SPACE_XXS),
        ]
        .spacing(0)
        .width(Length::Fill);

        // Pinned searches (only if non-empty)
        if !self.pinned_searches.is_empty() {
            col = col.push(pinned_searches_section(self));
            col = col.push(Space::new().height(SPACE_XXS));
        }

        col = col.push(widgets::compose_button(SidebarMessage::Compose));
        col = col.push(Space::new().height(SPACE_XS));
        col = col.push(nav_items(self));
        col = col.push(widgets::section_break());
        col = col.push(smart_folders(self));

        if show_labels {
            col = col.push(widgets::section_break::<SidebarMessage>());
            col = col.push(labels(self));
        }

        container(
            column![
                scrollable(col).spacing(SCROLLBAR_SPACING).height(Length::Fill),
                widgets::settings_button(SidebarMessage::ToggleSettings),
            ]
            .spacing(SPACE_XS),
        )
        .padding(PAD_SIDEBAR)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
    }
}

// ── Scope dropdown ──────────────────────────────────────

fn scope_dropdown(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let (trigger_icon, trigger_label): (DropdownIcon<'_>, &str) =
        match sidebar.selected_account {
            Some(idx) if sidebar.accounts.get(idx).is_some() => {
                let acc = &sidebar.accounts[idx];
                let name = acc.display_name.as_deref().unwrap_or(&acc.email);
                (DropdownIcon::Avatar(name), name)
            }
            _ => (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts"),
        };

    let mut entries: Vec<DropdownEntry<'_, SidebarMessage>> = Vec::new();

    entries.push(DropdownEntry {
        icon: DropdownIcon::Icon(icon::INBOX_CODEPOINT),
        label: "All Accounts",
        selected: sidebar.selected_account.is_none(),
        on_press: SidebarMessage::SelectAllAccounts,
    });

    for (idx, acc) in sidebar.accounts.iter().enumerate() {
        let name = acc.display_name.as_deref().unwrap_or(&acc.email);
        entries.push(DropdownEntry {
            icon: DropdownIcon::Avatar(name),
            label: name,
            selected: sidebar.selected_account == Some(idx),
            on_press: SidebarMessage::SelectAccount(idx),
        });
    }

    widgets::dropdown(
        trigger_icon,
        trigger_label,
        sidebar.scope_dropdown_open,
        SidebarMessage::ToggleScopeDropdown,
        entries,
    )
}

// ── Nav items ───────────────────────────────────────────

fn nav_items(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
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
                !matches!(f.id.as_str(), "SPAM" | "ALL_MAIL")
            } else {
                true
            }
        })
        .collect();

    let mut col = column![].spacing(SPACE_XXS);
    for f in &universal {
        let is_active = match &sidebar.selected_label {
            Some(lid) => lid == &f.id,
            None => f.id == "INBOX",
        };
        let on_press = if f.id == "INBOX" {
            SidebarMessage::SelectLabel(None)
        } else {
            SidebarMessage::SelectLabel(Some(f.id.clone()))
        };
        let nav_btn = widgets::nav_button(
            None,
            &f.name,
            is_active,
            widgets::NavSize::Compact,
            Some(f.unread_count),
            on_press,
        );
        let query_prefix = build_search_here_folder_prefix(&f.name, sidebar);
        col = col.push(
            mouse_area(nav_btn)
                .on_right_press(SidebarMessage::SearchHere(query_prefix)),
        );
    }
    col.into()
}

fn smart_folders(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let children: Vec<Element<'_, SidebarMessage>> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::SmartFolder))
        .map(|f| {
            widgets::nav_button(
                None,
                &f.name,
                sidebar.selected_label.as_deref() == Some(&f.id),
                widgets::NavSize::Compact,
                Some(f.unread_count),
                SidebarMessage::SelectLabel(Some(f.id.clone())),
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

fn labels(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar
        .nav_state
        .as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let account_labels: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountLabel))
        .collect();

    let has_hierarchy = account_labels.iter().any(|f| f.parent_id.is_some());

    let children: Vec<Element<'_, SidebarMessage>> = if has_hierarchy {
        render_label_tree(sidebar, &account_labels)
    } else {
        render_flat_labels(sidebar, &account_labels)
    };

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}

// ── Pinned searches ─────────────────────────────────────

fn pinned_searches_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let mut col = column![].spacing(SPACE_XXS);

    for ps in &sidebar.pinned_searches {
        col = col.push(pinned_search_card(
            ps,
            sidebar.active_pinned_search == Some(ps.id),
        ));
    }

    col.into()
}

/// Whether a pinned search's results are stale (> 1 hour old).
fn is_results_stale(updated_at: i64) -> bool {
    let Some(dt) = chrono::DateTime::from_timestamp(updated_at, 0) else {
        return true;
    };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(dt);
    delta.num_hours() >= 1
}

fn pinned_search_card(
    ps: &PinnedSearch,
    active: bool,
) -> Element<'_, SidebarMessage> {
    use iced::widget::text::Wrapping;

    let date_label = format_relative_time(ps.updated_at);
    let query_display = truncate_query(&ps.query, PINNED_SEARCH_QUERY_MAX_CHARS);
    let stale = is_results_stale(ps.updated_at);

    // Spec 1E.4: query is primary text, date is secondary
    let query_style: fn(&iced::Theme) -> text::Style = if active {
        text::primary
    } else {
        text::base
    };
    let date_style: fn(&iced::Theme) -> text::Style = if active {
        text::secondary
    } else {
        theme::TextClass::Muted.style()
    };

    let mut meta_row = row![
        text(date_label).size(TEXT_SM).style(date_style),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    // Staleness indicator: show when results are > 1 hour old
    if stale {
        meta_row = meta_row.push(
            text("outdated")
                .size(TEXT_XS)
                .style(theme::TextClass::Muted.style()),
        );
    }

    let text_col = column![
        text(query_display)
            .size(TEXT_MD)
            .style(query_style)
            .wrapping(Wrapping::None),
        meta_row,
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill);

    let mut actions = column![].spacing(SPACE_XXXS);

    let dismiss_btn = button(
        container(
            icon::x().size(ICON_XS).style(theme::TextClass::Muted.style()),
        )
        .center(Length::Shrink),
    )
    .on_press(SidebarMessage::DismissPinnedSearch(ps.id))
    .padding(SPACE_XXXS)
    .style(theme::ButtonClass::BareIcon.style());

    actions = actions.push(dismiss_btn);

    // Show refresh button when stale
    if stale {
        let refresh_btn = button(
            container(
                icon::refresh()
                    .size(ICON_XS)
                    .style(theme::TextClass::Muted.style()),
            )
            .center(Length::Shrink),
        )
        .on_press(SidebarMessage::RefreshPinnedSearch(ps.id))
        .padding(SPACE_XXXS)
        .style(theme::ButtonClass::BareIcon.style());

        actions = actions.push(refresh_btn);
    }

    let content = row![text_col, actions]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Start);

    button(
        container(content).padding(PAD_NAV_ITEM),
    )
    .on_press(SidebarMessage::SelectPinnedSearch(ps.id))
    .padding(0)
    .style(theme::ButtonClass::PinnedSearch { active }.style())
    .width(Length::Fill)
    .into()
}

/// Formats a unix timestamp as a relative time string (e.g. "5 min ago", "2 hours ago").
fn format_relative_time(timestamp: i64) -> String {
    let Some(dt) = chrono::DateTime::from_timestamp(timestamp, 0) else {
        return "Unknown".to_string();
    };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(dt);

    if delta.num_seconds() < 60 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        let m = delta.num_minutes();
        format!("{m} min ago")
    } else if delta.num_hours() < 24 {
        let h = delta.num_hours();
        format!("{h} hours ago")
    } else {
        let d = delta.num_days();
        format!("{d} days ago")
    }
}

/// Truncates a query string for display, adding ellipsis if needed.
pub fn truncate_query(query: &str, max_chars: usize) -> String {
    if query.len() <= max_chars {
        query.to_string()
    } else {
        format!("{}...", &query[..query.floor_char_boundary(max_chars)])
    }
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
        current_parent = id_to_folder
            .get(pid)
            .and_then(|f| f.parent_id.as_deref());
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
        if is_hidden_by_collapsed_ancestor(node.folder, &id_to_folder, &sidebar.collapsed_folders)
        {
            continue;
        }

        let has_children = labels
            .iter()
            .any(|f| f.parent_id.as_deref() == Some(&node.folder.id));
        let is_collapsed = sidebar.collapsed_folders.contains(&node.folder.id);
        let active = sidebar.selected_label.as_deref() == Some(&node.folder.id);
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
                .on_press(SidebarMessage::ToggleFolderExpand(
                    node.folder.id.clone(),
                ))
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
        .on_press(SidebarMessage::SelectLabel(Some(
            node.folder.id.clone(),
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
            let active = sidebar.selected_label.as_deref() == Some(&f.id);
            let label_btn = widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                active,
                SidebarMessage::SelectLabel(Some(f.id.clone())),
            );
            let query_prefix = build_search_here_prefix(&f.name, sidebar);
            mouse_area(label_btn)
                .on_right_press(SidebarMessage::SearchHere(query_prefix))
                .into()
        })
        .collect()
}

/// Build a scope query prefix for "Search here" from a label/folder name.
fn build_search_here_prefix(name: &str, sidebar: &Sidebar) -> String {
    match sidebar.selected_account {
        Some(idx) => {
            let account_name = sidebar
                .accounts
                .get(idx)
                .map(|a| a.display_name.as_deref().unwrap_or(&a.email))
                .unwrap_or("Unknown");
            format!(
                "account:{} label:{} ",
                quote_if_needed(account_name),
                quote_if_needed(name),
            )
        }
        None => format!("label:{} ", quote_if_needed(name)),
    }
}

/// Build a scope query prefix for universal folders (Inbox, Sent, etc.).
fn build_search_here_folder_prefix(
    folder_name: &str,
    sidebar: &Sidebar,
) -> String {
    match sidebar.selected_account {
        Some(idx) => {
            let account_name = sidebar
                .accounts
                .get(idx)
                .map(|a| a.display_name.as_deref().unwrap_or(&a.email))
                .unwrap_or("Unknown");
            format!(
                "account:{} in:{} ",
                quote_if_needed(account_name),
                folder_name.to_lowercase(),
            )
        }
        None => format!("in:{} ", folder_name.to_lowercase()),
    }
}

/// Wrap a value in quotes if it contains spaces.
fn quote_if_needed(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}
