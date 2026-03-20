use iced::widget::{column, container, row, scrollable, text, text_input, Space};
use iced::{Color, Element, Length, Task};

use crate::component::Component;
use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    SelectThread(usize),
    /// The search bar text changed.
    SearchInput(String),
    /// The user pressed Enter in the search bar.
    SearchSubmit,
}

/// Events the thread list emits upward to the App.
#[derive(Debug, Clone)]
pub enum ThreadListEvent {
    ThreadSelected(usize),
    /// The search query text changed (propagated to App for debounce).
    SearchQueryChanged(String),
    /// The user pressed Enter — execute search immediately.
    SearchExecute,
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
        }
    }

    pub fn set_threads(&mut self, threads: Vec<Thread>) {
        self.threads = threads;
    }

    pub fn set_context(&mut self, folder_name: String, scope_name: String) {
        self.folder_name = folder_name;
        self.scope_name = scope_name;
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
                (Task::none(), Some(ThreadListEvent::SearchQueryChanged(query)))
            }
            ThreadListMessage::SearchSubmit => {
                (Task::none(), Some(ThreadListEvent::SearchExecute))
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

        let body: Element<'_, ThreadListMessage> = if self.threads.is_empty() {
            let (title, subtitle) = match self.mode {
                ThreadListMode::Folder => ("No conversations", "This folder is empty"),
                ThreadListMode::Search => ("No results", "Try a different search"),
            };
            widgets::empty_placeholder(title, subtitle)
        } else {
            thread_list_body(&self.threads, self.selected_thread)
        };

        container(
            column![header, body]
                .spacing(0)
                .width(Length::Fill),
        )
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
    let search_input = text_input("Search...", search_query)
        .id("search-bar")
        .on_input(ThreadListMessage::SearchInput)
        .on_submit(ThreadListMessage::SearchSubmit)
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
        ThreadListMode::Search => row![
            text(format!("{thread_count} results"))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .align_y(iced::Alignment::Center)
        .into(),
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
