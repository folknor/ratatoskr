use iced::widget::{
    button, column, container, row, scrollable, stack, text, text_input, Space,
};
use iced::{Color, Element, Length, Task};
use ratatoskr_smart_folder::{CursorContext, analyze_cursor_context};

use crate::component::Component;
use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;
use crate::ui::widgets;

// ── Typeahead types ────────────────────────────────────

/// State for the operator typeahead popup.
#[derive(Debug, Clone, Default)]
pub struct TypeaheadState {
    /// Whether the popup is visible.
    pub visible: bool,
    /// The operator context that triggered the popup.
    pub context: Option<CursorContext>,
    /// Matching items from the data source.
    pub items: Vec<TypeaheadItem>,
    /// Currently highlighted item index.
    pub selected: usize,
}

/// A single item in the typeahead suggestion list.
#[derive(Debug, Clone)]
pub struct TypeaheadItem {
    /// Display label (e.g., "Alice Smith").
    pub label: String,
    /// Secondary text (e.g., "asmith@corp.com").
    pub detail: Option<String>,
    /// The value to insert into the query when selected.
    pub insert_value: String,
}

/// Direction for keyboard navigation in the typeahead popup.
#[derive(Debug, Clone)]
pub enum TypeaheadDirection {
    Up,
    Down,
}

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    SelectThread(usize),
    /// The search bar text changed.
    SearchInput(String),
    /// The user pressed Enter in the search bar.
    SearchSubmit,
    /// Undo the last search bar edit.
    SearchUndo,
    /// Redo a previously undone search bar edit.
    SearchRedo,
    /// Move selection up by one.
    SelectPrevious,
    /// Move selection down by one.
    SelectNext,
    /// Jump to first thread.
    SelectFirst,
    /// Jump to last thread.
    SelectLast,
    /// Open/activate the selected thread (Enter).
    ActivateSelected,
    /// Deselect current thread (Escape).
    Deselect,
    /// Widen search scope to "All" accounts.
    WidenSearchScope,
    /// User selected a typeahead suggestion by index.
    TypeaheadSelect(usize),
    /// User dismissed the typeahead popup.
    TypeaheadDismiss,
    /// Arrow key navigation within typeahead.
    TypeaheadNavigate(TypeaheadDirection),
    /// Typeahead items loaded from async data source.
    TypeaheadItemsLoaded(Vec<TypeaheadItem>),
}

/// Events the thread list emits upward to the App.
#[derive(Debug, Clone)]
pub enum ThreadListEvent {
    ThreadSelected(usize),
    /// The search query text changed (propagated to App for debounce).
    SearchQueryChanged(String),
    /// The user pressed Enter — execute search immediately.
    SearchExecute,
    /// Thread deselected.
    ThreadDeselected,
    /// Undo search bar text.
    SearchUndo,
    /// Redo search bar text.
    SearchRedo,
    /// User clicked "All" to widen search scope.
    WidenSearchScope,
    /// Typeahead needs suggestions for an operator value.
    TypeaheadQuery {
        operator: String,
        partial_value: String,
    },
}

// ── Thread list mode ───────────────────────────────────

/// What the thread list is currently displaying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadListMode {
    /// Browsing a folder or label — threads loaded from scoped DB query.
    Folder,
    /// Displaying search results — threads came from the unified search pipeline.
    Search,
}

// ── State ──────────────────────────────────────────────

pub struct ThreadList {
    pub threads: Vec<Thread>,
    pub selected_thread: Option<usize>,
    pub folder_name: String,
    pub scope_name: String,
    /// Current display mode (folder view vs search results).
    pub mode: ThreadListMode,
    /// The search query string, set by App before view() is called.
    pub search_query: String,
    /// Operator typeahead popup state.
    pub typeahead: TypeaheadState,
}

impl ThreadList {
    pub fn new() -> Self {
        Self {
            threads: Vec::new(),
            selected_thread: None,
            folder_name: "Inbox".to_string(),
            scope_name: "All".to_string(),
            mode: ThreadListMode::Folder,
            search_query: String::new(),
            typeahead: TypeaheadState::default(),
        }
    }

    pub fn set_threads(&mut self, threads: Vec<Thread>) {
        self.threads = threads;
    }

    pub fn set_context(&mut self, folder_name: String, scope_name: String) {
        self.folder_name = folder_name;
        self.scope_name = scope_name;
    }

    /// Move selection to the next thread (down), wrapping if needed.
    fn select_next(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            return None;
        }
        let next = match self.selected_thread {
            Some(i) if i + 1 < self.threads.len() => i + 1,
            Some(_) => return None, // already at end
            None => 0,
        };
        self.selected_thread = Some(next);
        Some(ThreadListEvent::ThreadSelected(next))
    }

    /// Accept the currently highlighted typeahead item and insert it
    /// into the search query.
    fn accept_typeahead_selection(
        &mut self,
    ) -> (Task<ThreadListMessage>, Option<ThreadListEvent>) {
        let idx = self.typeahead.selected;
        let Some(item) = self.typeahead.items.get(idx) else {
            self.typeahead.visible = false;
            return (Task::none(), None);
        };
        let Some(ref ctx) = self.typeahead.context else {
            self.typeahead.visible = false;
            return (Task::none(), None);
        };

        let new_query = apply_typeahead_selection(&self.search_query, ctx, item);
        self.typeahead.visible = false;
        self.typeahead.items.clear();
        self.typeahead.context = None;
        self.search_query.clone_from(&new_query);

        (
            Task::none(),
            Some(ThreadListEvent::SearchQueryChanged(new_query)),
        )
    }

    /// Move selection to the previous thread (up).
    fn select_previous(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            return None;
        }
        let prev = match self.selected_thread {
            Some(0) => return None, // already at start
            Some(i) => i - 1,
            None => 0,
        };
        self.selected_thread = Some(prev);
        Some(ThreadListEvent::ThreadSelected(prev))
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for ThreadList {
    type Message = ThreadListMessage;
    type Event = ThreadListEvent;

    fn update(
        &mut self,
        message: ThreadListMessage,
    ) -> (Task<ThreadListMessage>, Option<ThreadListEvent>) {
        match message {
            ThreadListMessage::SelectThread(idx) => {
                self.selected_thread = Some(idx);
                (Task::none(), Some(ThreadListEvent::ThreadSelected(idx)))
            }
            ThreadListMessage::SearchInput(query) => {
                // Analyze cursor context for typeahead.
                // The cursor is at the end of the new query string
                // (text_input on_input gives the full value after edit).
                let cursor_pos = query.len();
                let ctx = analyze_cursor_context(&query, cursor_pos);
                let event = match &ctx {
                    CursorContext::InsideOperator {
                        operator,
                        partial_value,
                        ..
                    } => {
                        self.typeahead.context = Some(ctx.clone());
                        self.typeahead.selected = 0;

                        // For static operators, populate items immediately.
                        // For dynamic operators, emit an event to query the DB.
                        match operator.as_str() {
                            "has" => {
                                self.typeahead.items =
                                    static_typeahead_items(HAS_PRESETS, partial_value);
                                self.typeahead.visible = !self.typeahead.items.is_empty();
                                None
                            }
                            "is" => {
                                self.typeahead.items =
                                    static_typeahead_items(IS_PRESETS, partial_value);
                                self.typeahead.visible = !self.typeahead.items.is_empty();
                                None
                            }
                            "in" => {
                                self.typeahead.items =
                                    static_typeahead_items(IN_PRESETS, partial_value);
                                self.typeahead.visible = !self.typeahead.items.is_empty();
                                None
                            }
                            "before" | "after" => {
                                self.typeahead.items =
                                    date_typeahead_items(partial_value);
                                self.typeahead.visible = !self.typeahead.items.is_empty();
                                None
                            }
                            "from" | "to" | "label" | "folder" | "account" => {
                                // Dynamic: emit event so App can query the DB.
                                self.typeahead.visible = false;
                                Some(ThreadListEvent::TypeaheadQuery {
                                    operator: operator.clone(),
                                    partial_value: partial_value.clone(),
                                })
                            }
                            _ => {
                                self.typeahead.visible = false;
                                self.typeahead.items.clear();
                                None
                            }
                        }
                    }
                    CursorContext::FreeText => {
                        self.typeahead.visible = false;
                        self.typeahead.items.clear();
                        self.typeahead.context = None;
                        None
                    }
                };

                // Always propagate the query change.
                let search_event = ThreadListEvent::SearchQueryChanged(query);
                // If we have a typeahead event, batch both; otherwise just search.
                if let Some(ta_event) = event {
                    // We can only return one event from update(). The search
                    // query changed event is critical, so return that.
                    // Typeahead query event will be emitted via the App's
                    // handle_search_query_changed which re-analyzes.
                    // Actually, we need both. Let's prioritize the search change
                    // and have the App's handler trigger typeahead loading.
                    _ = ta_event;
                    (Task::none(), Some(search_event))
                } else {
                    (Task::none(), Some(search_event))
                }
            }
            ThreadListMessage::SearchSubmit => {
                // Accept typeahead selection if popup is visible.
                if self.typeahead.visible && !self.typeahead.items.is_empty() {
                    return self.accept_typeahead_selection();
                }
                self.typeahead.visible = false;
                (Task::none(), Some(ThreadListEvent::SearchExecute))
            }
            ThreadListMessage::SearchUndo => {
                (Task::none(), Some(ThreadListEvent::SearchUndo))
            }
            ThreadListMessage::SearchRedo => {
                (Task::none(), Some(ThreadListEvent::SearchRedo))
            }
            ThreadListMessage::SelectNext => {
                let event = self.select_next();
                (Task::none(), event)
            }
            ThreadListMessage::SelectPrevious => {
                let event = self.select_previous();
                (Task::none(), event)
            }
            ThreadListMessage::SelectFirst => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                self.selected_thread = Some(0);
                (Task::none(), Some(ThreadListEvent::ThreadSelected(0)))
            }
            ThreadListMessage::SelectLast => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                let last = self.threads.len() - 1;
                self.selected_thread = Some(last);
                (Task::none(), Some(ThreadListEvent::ThreadSelected(last)))
            }
            ThreadListMessage::ActivateSelected => {
                if let Some(idx) = self.selected_thread {
                    (Task::none(), Some(ThreadListEvent::ThreadSelected(idx)))
                } else {
                    (Task::none(), None)
                }
            }
            ThreadListMessage::Deselect => {
                if self.typeahead.visible {
                    self.typeahead.visible = false;
                    return (Task::none(), None);
                }
                self.selected_thread = None;
                (Task::none(), Some(ThreadListEvent::ThreadDeselected))
            }
            ThreadListMessage::WidenSearchScope => {
                (Task::none(), Some(ThreadListEvent::WidenSearchScope))
            }
            ThreadListMessage::TypeaheadSelect(idx) => {
                self.typeahead.selected = idx;
                self.accept_typeahead_selection()
            }
            ThreadListMessage::TypeaheadDismiss => {
                self.typeahead.visible = false;
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadNavigate(direction) => {
                if !self.typeahead.visible || self.typeahead.items.is_empty() {
                    return (Task::none(), None);
                }
                match direction {
                    TypeaheadDirection::Up => {
                        self.typeahead.selected =
                            self.typeahead.selected.saturating_sub(1);
                    }
                    TypeaheadDirection::Down => {
                        let max = self.typeahead.items.len().saturating_sub(1);
                        self.typeahead.selected =
                            (self.typeahead.selected + 1).min(max);
                    }
                }
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadItemsLoaded(items) => {
                self.typeahead.items = items;
                self.typeahead.selected = 0;
                self.typeahead.visible = !self.typeahead.items.is_empty();
                (Task::none(), None)
            }
        }
    }

    fn view(&self) -> Element<'_, ThreadListMessage> {
        let header = thread_list_header(
            &self.folder_name,
            &self.scope_name,
            &self.search_query,
            &self.mode,
            self.threads.len(),
        );

        let typeahead_overlay = typeahead_popup(&self.typeahead);

        let body: Element<'_, ThreadListMessage> = if self.threads.is_empty() {
            let (title, subtitle) = match self.mode {
                ThreadListMode::Folder => ("No conversations", "This folder is empty"),
                ThreadListMode::Search => ("No results", "Try a different search"),
            };
            widgets::empty_placeholder(title, subtitle)
        } else {
            thread_list_body(&self.threads, self.selected_thread)
        };

        // Stack the typeahead popup over the body, anchored below the header.
        let content = column![
            header,
            stack![
                body,
                // Typeahead floats at the top of this stack region.
                container(typeahead_overlay)
                    .width(Length::Fill)
                    .padding([0.0, SPACE_SM]),
            ]
        ]
        .spacing(0)
        .width(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::ContainerClass::Base.style())
            .into()
    }
}

// ── View helpers ────────────────────────────────────────

fn thread_list_header<'a>(
    folder_name: &'a str,
    scope_name: &'a str,
    search_query: &'a str,
    mode: &ThreadListMode,
    thread_count: usize,
) -> Element<'a, ThreadListMessage> {
    let search_input = undoable_text_input("Search...", search_query)
        .id("search-bar")
        .on_input(ThreadListMessage::SearchInput)
        .on_submit(ThreadListMessage::SearchSubmit)
        .on_undo(ThreadListMessage::SearchUndo)
        .on_redo(ThreadListMessage::SearchRedo)
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let context_row: Element<'a, ThreadListMessage> = match mode {
        ThreadListMode::Folder => row![
            text(folder_name)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            Space::new().width(Length::Fill),
            text(scope_name)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .align_y(iced::Alignment::Center)
        .into(),
        ThreadListMode::Search => {
            let results_text = text(format!("{thread_count} results"))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style());

            let all_link = button(
                text("All")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(ThreadListMessage::WidenSearchScope)
            .padding(0)
            .style(theme::ButtonClass::Ghost.style());

            row![
                results_text,
                Space::new().width(Length::Fill),
                all_link,
            ]
            .align_y(iced::Alignment::Center)
            .into()
        }
    };

    container(
        column![search_input, context_row].spacing(SPACE_XXS),
    )
    .padding(PAD_PANEL_HEADER)
    .into()
}

fn thread_list_body<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
) -> Element<'a, ThreadListMessage> {
    let mut list = column![].spacing(0);
    for (i, thread) in threads.iter().enumerate() {
        let label_colors: &[(Color,)] = &[];
        list = list.push(widgets::thread_card(
            thread,
            i,
            selected_thread == Some(i),
            label_colors,
            ThreadListMessage::SelectThread,
        ));
    }
    scrollable(list)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill)
        .into()
}

// ── Typeahead popup ────────────────────────────────────

fn typeahead_popup(state: &TypeaheadState) -> Element<'_, ThreadListMessage> {
    if !state.visible || state.items.is_empty() {
        return Space::new().width(0).height(0).into();
    }

    let mut list = column![].spacing(0);
    for (i, item) in state.items.iter().enumerate() {
        let is_selected = i == state.selected;
        let item_row = typeahead_item_view(item, is_selected);
        list = list.push(
            button(item_row)
                .on_press(ThreadListMessage::TypeaheadSelect(i))
                .style(
                    theme::ButtonClass::Dropdown {
                        selected: is_selected,
                    }
                    .style(),
                )
                .width(Length::Fill)
                .padding(PAD_DROPDOWN),
        );
    }

    // "Keep as text" option at the bottom.
    let keep_text = button(
        text("Keep as text")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .on_press(ThreadListMessage::TypeaheadDismiss)
    .style(theme::ButtonClass::Ghost.style())
    .width(Length::Fill)
    .padding(PAD_DROPDOWN);

    container(column![list, keep_text].spacing(0))
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill)
        .max_height(TYPEAHEAD_MAX_HEIGHT)
        .into()
}

fn typeahead_item_view<'a>(
    item: &'a TypeaheadItem,
    _is_selected: bool,
) -> Element<'a, ThreadListMessage> {
    let label = text(&item.label).size(TEXT_MD);

    if let Some(ref detail) = item.detail {
        row![
            label,
            Space::new().width(Length::Fill),
            text(detail)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(SPACE_XS)
        .into()
    } else {
        container(label)
            .width(Length::Fill)
            .align_y(iced::Alignment::Center)
            .into()
    }
}

// ── Typeahead selection insertion ──────────────────────

/// Replace the partial operator value in the query with the selected item.
fn apply_typeahead_selection(
    query: &str,
    context: &CursorContext,
    item: &TypeaheadItem,
) -> String {
    if let CursorContext::InsideOperator {
        value_start,
        value_end,
        ..
    } = context
    {
        let value = if item.insert_value.contains(' ') {
            format!("\"{}\" ", item.insert_value)
        } else {
            format!("{} ", item.insert_value)
        };
        format!("{}{}{}", &query[..*value_start], value, &query[*value_end..])
    } else {
        query.to_string()
    }
}

// ── Static typeahead presets ───────────────────────────

/// Preset label/value pairs for `has:` operator.
const HAS_PRESETS: &[(&str, &str)] = &[
    ("attachment", "attachment"),
    ("image", "image"),
    ("pdf", "pdf"),
    ("document", "document"),
    ("spreadsheet", "spreadsheet"),
    ("archive", "archive"),
    ("video", "video"),
    ("audio", "audio"),
    ("calendar", "calendar"),
];

/// Preset label/value pairs for `is:` operator.
const IS_PRESETS: &[(&str, &str)] = &[
    ("read", "read"),
    ("unread", "unread"),
    ("starred", "starred"),
    ("snoozed", "snoozed"),
    ("pinned", "pinned"),
    ("muted", "muted"),
    ("tagged", "tagged"),
];

/// Preset label/value pairs for `in:` operator.
const IN_PRESETS: &[(&str, &str)] = &[
    ("inbox", "inbox"),
    ("sent", "sent"),
    ("drafts", "drafts"),
    ("trash", "trash"),
    ("spam", "spam"),
    ("starred", "starred"),
    ("snoozed", "snoozed"),
];

/// Date presets for `before:` and `after:` operators.
const DATE_PRESETS: &[(&str, &str)] = &[
    ("Today", "0"),
    ("Yesterday", "-1"),
    ("Last 7 days", "-7"),
    ("Last 30 days", "-30"),
    ("Last 3 months", "-90"),
    ("Last year", "-365"),
];

/// Filter static preset items by partial value match.
fn static_typeahead_items(
    presets: &[(&str, &str)],
    partial: &str,
) -> Vec<TypeaheadItem> {
    let lower = partial.to_ascii_lowercase();
    presets
        .iter()
        .filter(|(label, _)| {
            lower.is_empty() || label.to_ascii_lowercase().contains(&lower)
        })
        .map(|(label, value)| TypeaheadItem {
            label: (*label).to_owned(),
            detail: None,
            insert_value: (*value).to_owned(),
        })
        .collect()
}

/// Filter date presets by partial value match.
fn date_typeahead_items(partial: &str) -> Vec<TypeaheadItem> {
    let lower = partial.to_ascii_lowercase();
    DATE_PRESETS
        .iter()
        .filter(|(label, value)| {
            lower.is_empty()
                || label.to_ascii_lowercase().contains(&lower)
                || value.contains(&lower)
        })
        .map(|(label, value)| TypeaheadItem {
            label: (*label).to_owned(),
            detail: Some((*value).to_owned()),
            insert_value: (*value).to_owned(),
        })
        .collect()
}
