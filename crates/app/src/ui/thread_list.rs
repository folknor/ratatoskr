use iced::widget::{column, container, row, scrollable, text, Space};
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
}

/// Events the thread list emits upward to the App.
#[derive(Debug, Clone)]
pub enum ThreadListEvent {
    ThreadSelected(usize),
}

// ── State ──────────────────────────────────────────────

pub struct ThreadList {
    pub threads: Vec<Thread>,
    pub selected_thread: Option<usize>,
    pub folder_name: String,
    pub scope_name: String,
}

impl ThreadList {
    pub fn new() -> Self {
        Self {
            threads: Vec::new(),
            selected_thread: None,
            folder_name: "Inbox".to_string(),
            scope_name: "All".to_string(),
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
        }
    }

    fn view(&self) -> Element<'_, ThreadListMessage> {
        let header = thread_list_header(&self.folder_name, &self.scope_name);

        let body: Element<'_, ThreadListMessage> = if self.threads.is_empty() {
            widgets::empty_placeholder("No conversations", "This folder is empty")
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
) -> Element<'a, ThreadListMessage> {
    container(
        column![
            container(text("Search...").size(TEXT_MD).style(theme::TextClass::Tertiary.style()))
                .padding(PAD_INPUT)
                .width(Length::Fill)
                .style(theme::ContainerClass::Elevated.style()),
            row![
                text(folder_name).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
                Space::new().width(Length::Fill),
                text(scope_name).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            ]
            .align_y(iced::Alignment::Center),
        ]
        .spacing(SPACE_XXS),
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
