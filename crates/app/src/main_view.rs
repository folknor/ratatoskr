use crate::app::{App, AppMode, DIVIDER_WIDTH, Divider};
use crate::command_dispatch;
use crate::component::Component;
use crate::message::Message;
use crate::pop_out::{self, PopOutWindow};
use crate::ui;
use crate::ui::add_account::AddAccountWizard;
use crate::ui::layout::{
    READING_PANE_MIN_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH,
};
use cmdk::current_platform;
use iced::widget::{Space, column, container, mouse_area, row, stack};
use iced::{Element, Length, Point, Task};

impl App {
    pub(crate) fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        if window_id == self.main_window_id {
            return self.view_main_window();
        }

        if let Some(pop_out) = self.pop_out_windows.get(&window_id) {
            return match pop_out {
                PopOutWindow::MessageView(state) => pop_out::message_view::view_message_window(
                    window_id,
                    state,
                    &self.thread_list.bimi_cache,
                ),
                PopOutWindow::Compose(state) => {
                    pop_out::compose::view_compose_window(window_id, state)
                }
                PopOutWindow::Calendar => ui::calendar::calendar_layout(&self.calendar)
                    .map(|m| Message::Calendar(Box::new(m))),
            };
        }

        ui::widgets::empty_placeholder("Window not found", "")
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn view_main_window(&self) -> Element<'_, Message> {
        if let Some(ref wizard) = self.add_account_wizard {
            if self.no_accounts {
                return self.view_first_launch_modal(wizard);
            }
            return self.view_with_add_account_modal(wizard);
        }

        if self.show_settings {
            let settings_view = self.settings.view().map(Message::Settings);
            return container(settings_view)
                .height(Length::Fill)
                .width(Length::Fill)
                .into();
        }

        let layout = match self.app_mode {
            AppMode::Calendar => {
                let calendar_view = ui::calendar::calendar_layout(&self.calendar)
                    .map(|m| Message::Calendar(Box::new(m)));
                row![calendar_view].height(Length::Fill)
            }
            AppMode::Mail => {
                let sidebar = container(self.sidebar.view().map(Message::Sidebar))
                    .width(SIDEBAR_MIN_WIDTH)
                    .height(Length::Fill);

                let is_chat = self.active_chat.is_some();

                if is_chat {
                    // Chat view: sidebar + full-width chat timeline
                    let chat_view = if let Some(ref timeline) = self.chat_timeline {
                        container(timeline.view().map(Message::ChatTimeline))
                            .width(Length::Fill)
                            .height(Length::Fill)
                    } else {
                        container(
                            iced::widget::text("No chat selected")
                                .style(ui::theme::TextClass::Muted.style()),
                        )
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                        .width(Length::Fill)
                        .height(Length::Fill)
                    };

                    let status_bar = self.status_bar_view();
                    let content_area = column![chat_view, status_bar,];

                    let divider_sidebar = self.build_divider(Divider::Sidebar);
                    row![sidebar, divider_sidebar, content_area].height(Length::Fill)
                } else {
                    // Normal mail view: sidebar + thread list + reading pane
                    let thread_list = container(self.thread_list.view().map(Message::ThreadList))
                        .width(self.thread_list_width)
                        .height(Length::Fill);

                    let divider_thread = self.build_divider(Divider::ThreadList);

                    let ctx = command_dispatch::build_context(self);
                    let reading_pane = container(self.reading_pane.view_with_commands(
                        &self.registry,
                        &self.binding_table,
                        &ctx,
                    ))
                    .width(Length::Fill)
                    .height(Length::Fill);

                    let rs_data = ui::right_sidebar::RightSidebarData {
                        calendar: &self.calendar,
                        threads: &self.thread_list.threads,
                    };
                    let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open, &rs_data);

                    let status_bar = self.status_bar_view();
                    let content_area = column![
                        row![thread_list, divider_thread, reading_pane, right_sidebar]
                            .height(Length::Fill),
                        status_bar,
                    ];

                    row![sidebar, content_area].height(Length::Fill)
                }
            }
        };

        let full_layout = column![layout];

        let main_layout: Element<'_, Message> = if self.dragging.is_some() {
            mouse_area(full_layout)
                .on_move(Message::DividerDragMove)
                .on_release(Message::DividerDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            full_layout.into()
        };

        if self.palette.is_open() {
            let palette_widget = self.palette.view().map(Message::Palette);

            let palette_positioned = container(palette_widget)
                .width(Length::Fill)
                .padding(iced::Padding {
                    top: ui::layout::PALETTE_TOP_OFFSET,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .align_x(iced::Alignment::Center);

            ui::modal_overlay::modal_overlay(
                main_layout,
                palette_positioned,
                ui::modal_overlay::ModalSurface::Modal,
                Message::Noop,
            )
        } else if let Some(ref pending) = self.pending_chord {
            let chord_display = pending.first.display(current_platform());
            let indicator = ui::palette::chord_indicator::<Message>(&chord_display);
            let indicator_positioned = container(indicator)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::Alignment::End);
            stack![main_layout, indicator_positioned].into()
        } else {
            main_layout
        }
    }

    pub(crate) fn view_first_launch_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let modal_content = wizard.view().map(Message::AddAccount);

        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::Elevated.style());

        container(modal)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(ui::theme::ContainerClass::Content.style())
            .into()
    }

    pub(crate) fn view_with_add_account_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let base_layout = self.view_main_layout();

        let modal_content = wizard.view().map(Message::AddAccount);

        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::Elevated.style());

        ui::modal_overlay::modal_overlay(
            base_layout,
            modal,
            ui::modal_overlay::ModalSurface::Modal,
            Message::Noop,
        )
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn view_main_layout(&self) -> Element<'_, Message> {
        let sidebar = container(self.sidebar.view().map(Message::Sidebar))
            .width(self.sidebar_width)
            .height(Length::Fill);
        let divider_sidebar = self.build_divider(Divider::Sidebar);
        let thread_list = container(self.thread_list.view().map(Message::ThreadList))
            .width(self.thread_list_width)
            .height(Length::Fill);
        let divider_thread = self.build_divider(Divider::ThreadList);
        let ctx = command_dispatch::build_context(self);
        let reading_pane = container(self.reading_pane.view_with_commands(
            &self.registry,
            &self.binding_table,
            &ctx,
        ))
        .width(Length::Fill)
        .height(Length::Fill);
        let rs_data = ui::right_sidebar::RightSidebarData {
            calendar: &self.calendar,
            threads: &self.thread_list.threads,
        };
        let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open, &rs_data);
        let layout = row![
            sidebar,
            divider_sidebar,
            thread_list,
            divider_thread,
            reading_pane,
            right_sidebar
        ]
        .height(Length::Fill);
        let status_bar = self.status_bar_view();
        column![layout, status_bar].into()
    }

    pub(crate) fn handle_divider_drag(&mut self, point: Point) -> Task<Message> {
        // Available width for the three main panels (excludes right sidebar
        // when open, and both dividers).
        let right_sidebar_used = if self.right_sidebar_open {
            ui::layout::RIGHT_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let available = self.window.width - 2.0 * DIVIDER_WIDTH - right_sidebar_used;

        match self.dragging {
            Some(Divider::Sidebar) => {
                // max: leave room for thread list min + reading pane min
                let max_sidebar = available - THREAD_LIST_MIN_WIDTH - READING_PANE_MIN_WIDTH;
                self.sidebar_width = point
                    .x
                    .clamp(SIDEBAR_MIN_WIDTH, max_sidebar.max(SIDEBAR_MIN_WIDTH));
            }
            Some(Divider::ThreadList) => {
                // max: leave room for reading pane min
                let max_thread_list = available - self.sidebar_width - READING_PANE_MIN_WIDTH;
                let new_width = (point.x - self.sidebar_width - DIVIDER_WIDTH).clamp(
                    THREAD_LIST_MIN_WIDTH,
                    max_thread_list.max(THREAD_LIST_MIN_WIDTH),
                );
                self.thread_list_width = new_width;
            }
            None => {}
        }
        Task::none()
    }

    /// Clamp sidebar and thread-list widths so that all three panels
    /// respect their minimums at the current window size.  Called after
    /// every main-window resize.
    pub(crate) fn clamp_panel_widths(&mut self) {
        let right_sidebar_used = if self.right_sidebar_open {
            ui::layout::RIGHT_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let available = self.window.width - 2.0 * DIVIDER_WIDTH - right_sidebar_used;

        // 1. Ensure sidebar doesn't exceed what leaves room for the other
        //    two panels at their minimums.
        let max_sidebar =
            (available - THREAD_LIST_MIN_WIDTH - READING_PANE_MIN_WIDTH).max(SIDEBAR_MIN_WIDTH);
        self.sidebar_width = self.sidebar_width.clamp(SIDEBAR_MIN_WIDTH, max_sidebar);

        // 2. Ensure thread list doesn't exceed what leaves room for the
        //    reading pane at its minimum.
        let max_thread_list =
            (available - self.sidebar_width - READING_PANE_MIN_WIDTH).max(THREAD_LIST_MIN_WIDTH);
        self.thread_list_width = self
            .thread_list_width
            .clamp(THREAD_LIST_MIN_WIDTH, max_thread_list);
    }

    pub(crate) fn build_divider(&self, divider: Divider) -> Element<'_, Message> {
        let class = if self.hovered_divider == Some(divider) || self.dragging == Some(divider) {
            ui::theme::ContainerClass::DividerHover
        } else {
            ui::theme::ContainerClass::Divider
        };
        mouse_area(
            container("")
                .width(DIVIDER_WIDTH)
                .height(Length::Fill)
                .style(class.style()),
        )
        .on_press(Message::DividerDragStart(divider))
        .on_release(Message::DividerDragEnd)
        .on_enter(Message::DividerHover(divider))
        .on_exit(Message::DividerUnhover)
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .into()
    }

    /// Render the status bar, respecting the `sync_status_bar` setting.
    /// When the setting is off, returns an empty zero-height element.
    pub(crate) fn status_bar_view(&self) -> Element<'_, Message> {
        if self.settings.sync_status_bar {
            self.status_bar.view().map(Message::StatusBar)
        } else {
            Space::new().width(0).height(0).into()
        }
    }
}
