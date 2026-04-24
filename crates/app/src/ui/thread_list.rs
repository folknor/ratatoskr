use std::collections::HashSet;
use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Color, Element, Length, Padding, Task};

use crate::component::Component;
use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

// ── Auto-advance direction ─────────────────────────────

/// Which direction to advance after an email action removes the current thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAdvanceDirection {
    /// Select the next thread (below) after action.
    Next,
    /// Select the previous thread (above) after action.
    Previous,
}

// ── Typeahead types ───────────────────────────────────

/// A single typeahead suggestion item.
#[derive(Debug, Clone)]
pub struct TypeaheadItem {
    pub label: String,
    pub detail: Option<String>,
    pub insert_value: String,
}

/// Direction for navigating typeahead suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeaheadDirection {
    Up,
    Down,
}

/// State for the typeahead suggestion dropdown.
#[derive(Debug)]
pub struct TypeaheadState {
    pub visible: bool,
    pub items: Vec<TypeaheadItem>,
    pub selected: usize,
    /// Generation counter for stale typeahead result detection.
    pub generation: rtsk::generation::GenerationCounter<rtsk::generation::Typeahead>,
}

impl Default for TypeaheadState {
    fn default() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            selected: 0,
            generation: rtsk::generation::GenerationCounter::new(),
        }
    }
}

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    SelectThread(usize),
    /// Ctrl+click: toggle a thread in/out of multi-selection.
    ToggleThread(usize),
    /// Shift+click: range-select from last selected to this index.
    RangeSelectThread(usize),
    /// Select all threads (Ctrl+A).
    SelectAll,
    /// Select from the current thread to the end of the list (Ctrl+Shift+A).
    SelectFromHere,
    /// Clear multi-selection.
    SelectNone,
    /// The search bar text changed.
    SearchInput(String),
    /// The user pressed Enter in the search bar.
    SearchSubmit,
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
    /// Auto-advance after an email action removed the actioned thread(s).
    AutoAdvance,
    /// Navigate typeahead suggestions up or down.
    TypeaheadNavigate(TypeaheadDirection),
    /// Typeahead suggestion items loaded from async query (carries generation).
    TypeaheadItemsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Typeahead>,
        Vec<TypeaheadItem>,
    ),
    /// User selected a typeahead suggestion by index.
    TypeaheadSelect(usize),
    /// Dismiss the typeahead dropdown.
    TypeaheadDismiss,
}

/// Events the thread list emits upward to the App.
#[derive(Debug, Clone)]
pub enum ThreadListEvent {
    ThreadSelected(usize),
    /// The search query text changed (propagated to App for debounce).
    SearchQueryChanged(String),
    /// The user pressed Enter - execute search immediately.
    SearchExecute,
    /// Thread deselected.
    ThreadDeselected,
    /// User clicked "All" to widen search scope.
    WidenSearchScope,
    /// Multi-selection changed - App may want to update action availability.
    MultiSelectionChanged(usize),
    /// Auto-advance selected a new thread after action.
    AutoAdvance {
        /// The index that was auto-selected (None if list is now empty).
        new_index: Option<usize>,
    },
    /// Batch action: apply email action to all selected thread indices.
    BatchAction(Vec<usize>),
    /// Typeahead operator query needs async data.
    TypeaheadQuery {
        operator: String,
        partial_value: String,
    },
    /// User selected a typeahead suggestion.
    TypeaheadSelected(usize),
    /// Search undo requested.
    SearchUndo,
    /// Search redo requested.
    SearchRedo,
}

// ── Thread list mode ───────────────────────────────────

/// What the thread list is currently displaying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadListMode {
    /// Browsing a folder or label - threads loaded from scoped DB query.
    Folder,
    /// Displaying search results - threads came from the unified search pipeline.
    Search,
}

// ── State ──────────────────────────────────────────────

pub struct ThreadList {
    pub threads: Vec<Thread>,
    pub selected_thread: Option<usize>,
    /// Multi-selection set (indices into `threads`). Empty when single-select.
    pub selected_threads: HashSet<usize>,
    /// The last index that was clicked/toggled, used as anchor for shift-click range select.
    pub last_selected_anchor: Option<usize>,
    pub folder_name: String,
    pub scope_name: String,
    /// Current display mode (folder view vs search results).
    pub mode: ThreadListMode,
    /// The search query string, set by App before view() is called.
    pub search_query: String,
    /// Direction to auto-advance after an email action removes a thread.
    pub auto_advance_direction: AutoAdvanceDirection,
    /// Typeahead suggestion state.
    pub typeahead: TypeaheadState,
    /// BIMI logo LRU cache - shared with App for cross-component access.
    pub bimi_cache: Arc<rtsk::bimi::BimiLruCache>,
}

impl ThreadList {
    pub fn new(bimi_cache: Arc<rtsk::bimi::BimiLruCache>) -> Self {
        Self {
            threads: Vec::new(),
            selected_thread: None,
            selected_threads: HashSet::new(),
            last_selected_anchor: None,
            folder_name: "Inbox".to_string(),
            scope_name: "All".to_string(),
            mode: ThreadListMode::Folder,
            search_query: String::new(),
            auto_advance_direction: AutoAdvanceDirection::Next,
            typeahead: TypeaheadState::default(),
            bimi_cache,
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn set_threads(&mut self, threads: Vec<Thread>) {
        self.threads = threads;
        // Clear multi-selection when thread list changes - stale indices.
        self.selected_threads.clear();
        self.last_selected_anchor = None;
    }

    pub fn set_context(&mut self, folder_name: String, scope_name: String) {
        self.folder_name = folder_name;
        self.scope_name = scope_name;
    }

    /// Clear multi-selection state, keeping only single selection.
    pub fn clear_multi_select(&mut self) {
        self.selected_threads.clear();
    }

    /// Number of selected threads (multi-select or single).
    pub fn selection_count(&self) -> usize {
        if self.selected_threads.is_empty() {
            if self.selected_thread.is_some() { 1 } else { 0 }
        } else {
            self.selected_threads.len()
        }
    }

    /// Returns all selected indices (multi-select or single).
    pub fn selected_indices(&self) -> Vec<usize> {
        if self.selected_threads.is_empty() {
            self.selected_thread.into_iter().collect()
        } else {
            let mut indices: Vec<usize> = self.selected_threads.iter().copied().collect();
            indices.sort_unstable();
            indices
        }
    }

    /// Whether a thread at the given index is selected (single or multi).
    fn is_selected(&self, idx: usize) -> bool {
        if self.selected_threads.is_empty() {
            self.selected_thread == Some(idx)
        } else {
            self.selected_threads.contains(&idx)
        }
    }

    /// Move selection to the next thread (down).
    fn select_next(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            return None;
        }
        self.clear_multi_select();
        let next = match self.selected_thread {
            Some(i) if i + 1 < self.threads.len() => i + 1,
            Some(_) => return None, // already at end
            None => 0,
        };
        self.selected_thread = Some(next);
        self.last_selected_anchor = Some(next);
        Some(ThreadListEvent::ThreadSelected(next))
    }

    /// Move selection to the previous thread (up).
    fn select_previous(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            return None;
        }
        self.clear_multi_select();
        let prev = match self.selected_thread {
            Some(0) => return None, // already at start
            Some(i) => i - 1,
            None => 0,
        };
        self.selected_thread = Some(prev);
        self.last_selected_anchor = Some(prev);
        Some(ThreadListEvent::ThreadSelected(prev))
    }

    /// Auto-advance: select the next (or previous) thread after the current
    /// selection was removed by an email action.
    fn auto_advance(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            self.selected_thread = None;
            self.clear_multi_select();
            return Some(ThreadListEvent::AutoAdvance { new_index: None });
        }

        let prev_idx = self.selected_thread.unwrap_or(0);
        let new_idx = match self.auto_advance_direction {
            AutoAdvanceDirection::Next => {
                if prev_idx < self.threads.len() {
                    prev_idx
                } else {
                    self.threads.len().saturating_sub(1)
                }
            }
            AutoAdvanceDirection::Previous => {
                if prev_idx > 0 {
                    prev_idx.saturating_sub(1)
                } else {
                    0
                }
            }
        };

        self.selected_thread = Some(new_idx);
        self.clear_multi_select();
        self.last_selected_anchor = Some(new_idx);
        Some(ThreadListEvent::AutoAdvance {
            new_index: Some(new_idx),
        })
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
                if self.selected_thread == Some(idx) && self.selected_threads.is_empty() {
                    return (Task::none(), None);
                }

                let had_multi_select = !self.selected_threads.is_empty();
                // Plain click: clear multi-select, select single.
                self.clear_multi_select();
                self.selected_thread = Some(idx);
                self.last_selected_anchor = Some(idx);
                if had_multi_select {
                    (
                        Task::none(),
                        Some(ThreadListEvent::MultiSelectionChanged(0)),
                    )
                } else {
                    (Task::none(), Some(ThreadListEvent::ThreadSelected(idx)))
                }
            }
            ThreadListMessage::ToggleThread(idx) => {
                // Ctrl+click: toggle individual thread in/out.
                if self.selected_threads.is_empty() {
                    // Entering multi-select: seed with current selection.
                    if let Some(prev) = self.selected_thread {
                        self.selected_threads.insert(prev);
                    }
                }
                if self.selected_threads.contains(&idx) {
                    self.selected_threads.remove(&idx);
                } else {
                    self.selected_threads.insert(idx);
                }
                self.last_selected_anchor = Some(idx);
                self.selected_thread = Some(idx);
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::RangeSelectThread(idx) => {
                // Shift+click: range select from anchor to idx.
                let anchor = self.last_selected_anchor.unwrap_or(0);
                let (start, end) = if anchor <= idx {
                    (anchor, idx)
                } else {
                    (idx, anchor)
                };
                if self.selected_threads.is_empty() {
                    if let Some(prev) = self.selected_thread {
                        self.selected_threads.insert(prev);
                    }
                }
                for i in start..=end {
                    self.selected_threads.insert(i);
                }
                self.selected_thread = Some(idx);
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectAll => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                self.selected_threads = (0..self.threads.len()).collect();
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectFromHere => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                let start = self
                    .selected_thread
                    .or_else(|| self.last_selected_anchor)
                    .unwrap_or(0);
                for idx in start..self.threads.len() {
                    self.selected_threads.insert(idx);
                }
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectNone => {
                self.clear_multi_select();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(0)),
                )
            }
            ThreadListMessage::AutoAdvance => {
                let event = self.auto_advance();
                (Task::none(), event)
            }
            ThreadListMessage::SearchInput(query) => (
                Task::none(),
                Some(ThreadListEvent::SearchQueryChanged(query)),
            ),
            ThreadListMessage::SearchSubmit => (Task::none(), Some(ThreadListEvent::SearchExecute)),
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
                self.clear_multi_select();
                self.selected_thread = Some(0);
                self.last_selected_anchor = Some(0);
                (Task::none(), Some(ThreadListEvent::ThreadSelected(0)))
            }
            ThreadListMessage::SelectLast => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                self.clear_multi_select();
                let last = self.threads.len() - 1;
                self.selected_thread = Some(last);
                self.last_selected_anchor = Some(last);
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
                self.selected_thread = None;
                self.clear_multi_select();
                (Task::none(), Some(ThreadListEvent::ThreadDeselected))
            }
            ThreadListMessage::WidenSearchScope => {
                (Task::none(), Some(ThreadListEvent::WidenSearchScope))
            }
            ThreadListMessage::TypeaheadNavigate(direction) => {
                if !self.typeahead.items.is_empty() {
                    match direction {
                        TypeaheadDirection::Up => {
                            if self.typeahead.selected > 0 {
                                self.typeahead.selected -= 1;
                            }
                        }
                        TypeaheadDirection::Down => {
                            if self.typeahead.selected + 1 < self.typeahead.items.len() {
                                self.typeahead.selected += 1;
                            }
                        }
                    }
                }
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadItemsLoaded(load_gen, items) => {
                if !self.typeahead.generation.is_current(load_gen) {
                    return (Task::none(), None); // stale
                }
                self.typeahead.items = items;
                self.typeahead.selected = 0;
                self.typeahead.visible = !self.typeahead.items.is_empty();
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadSelect(idx) => {
                self.typeahead.visible = false;
                (Task::none(), Some(ThreadListEvent::TypeaheadSelected(idx)))
            }
            ThreadListMessage::TypeaheadDismiss => {
                self.typeahead.visible = false;
                (Task::none(), None)
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn view(&self) -> Element<'_, ThreadListMessage> {
        let selection_count = self.selection_count();
        let header = thread_list_header(
            &self.folder_name,
            &self.scope_name,
            &self.search_query,
            &self.mode,
            self.threads.len(),
            selection_count,
            &self.typeahead,
        );

        let body: Element<'_, ThreadListMessage> = if self.threads.is_empty() {
            let (title, subtitle) = match self.mode {
                ThreadListMode::Folder => ("No conversations", "This folder is empty"),
                ThreadListMode::Search => ("No results", "Try a different search"),
            };
            widgets::empty_placeholder(title, subtitle)
        } else {
            thread_list_body(self)
        };

        container(column![header, body].spacing(0).width(Length::Fill))
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
    selection_count: usize,
    typeahead: &'a TypeaheadState,
) -> Element<'a, ThreadListMessage> {
    let search_input = text_input("Search...", search_query)
        .id("search-bar")
        .on_input(ThreadListMessage::SearchInput)
        .on_submit(ThreadListMessage::SearchSubmit)
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let context_row: Element<'a, ThreadListMessage> = if selection_count > 1 {
        // Multi-selection: show count and deselect link.
        let count_text = text(format!("{selection_count} selected"))
            .size(TEXT_SM)
            .style(theme::TextClass::Accent.style());

        let deselect_link = button(
            text("Deselect")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .on_press(ThreadListMessage::SelectNone)
        .padding(0)
        .style(theme::ButtonClass::Ghost.style());

        row![count_text, Space::new().width(Length::Fill), deselect_link,]
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        match mode {
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

                row![results_text, Space::new().width(Length::Fill), all_link,]
                    .align_y(iced::Alignment::Center)
                    .into()
            }
        }
    };

    let mut header_col = column![search_input, context_row].spacing(SPACE_XXS);

    // Typeahead popup
    if typeahead.visible && !typeahead.items.is_empty() {
        let mut list = column![].spacing(0);
        for (i, item) in typeahead.items.iter().enumerate() {
            let is_selected = i == typeahead.selected;
            let style = if is_selected {
                theme::ButtonClass::Nav { active: true }
            } else {
                theme::ButtonClass::Action
            };
            let mut item_row = row![text(&item.label).size(TEXT_SM),]
                .spacing(SPACE_XS)
                .align_y(iced::Alignment::Center);
            if let Some(ref detail) = item.detail {
                item_row = item_row.push(
                    text(detail)
                        .size(TEXT_XS)
                        .style(theme::TextClass::Tertiary.style()),
                );
            }
            list = list.push(
                button(
                    container(item_row)
                        .padding(Padding::from([SPACE_XXS, SPACE_SM]))
                        .width(Length::Fill),
                )
                .on_press(ThreadListMessage::TypeaheadSelect(i))
                .padding(0)
                .style(style.style())
                .width(Length::Fill),
            );
        }
        header_col = header_col.push(
            container(list)
                .style(theme::ContainerClass::Elevated.style())
                .width(Length::Fill),
        );
    }

    container(header_col).padding(PAD_PANEL_HEADER).into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn thread_list_body(state: &ThreadList) -> Element<'_, ThreadListMessage> {
    let mut list = column![].spacing(0);
    for (i, thread) in state.threads.iter().enumerate() {
        let label_colors: &[(Color,)] = &[];
        // Look up BIMI logo for the sender's domain.
        let bimi_logo = thread
            .from_address
            .as_deref()
            .and_then(|addr| addr.rsplit('@').next())
            .and_then(|domain| {
                // get() returns None on miss, Some(None) for negative cache,
                // Some(Some(path)) for a cached logo.
                match state.bimi_cache.get(domain) {
                    Some(Some(path)) if path.exists() => Some(path),
                    _ => None,
                }
            });
        list = list.push(widgets::thread_card(
            thread,
            i,
            state.is_selected(i),
            label_colors,
            bimi_logo.as_deref(),
            ThreadListMessage::SelectThread,
        ));
    }
    scrollable(list)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill)
        .into()
}
