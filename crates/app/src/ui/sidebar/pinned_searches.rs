use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::db::PinnedSearch;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;

use super::{PINNED_SEARCH_QUERY_MAX_CHARS, Sidebar, SidebarMessage, truncate_query};

// ── Pinned searches ─────────────────────────────────────

pub(super) fn pinned_searches_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let mut col = column![].spacing(SPACE_XXS);

    for ps in &sidebar.pinned_searches {
        col = col.push(pinned_search_card(
            sidebar,
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

fn pinned_search_card<'a>(
    sidebar: &'a Sidebar,
    ps: &'a PinnedSearch,
    active: bool,
) -> Element<'a, SidebarMessage> {
    use iced::widget::text::Wrapping;

    let date_label = format_relative_time(ps.updated_at);
    let query_display = truncate_query(&ps.query, PINNED_SEARCH_QUERY_MAX_CHARS);
    let stale = is_results_stale(ps.updated_at);
    let scope_label = pinned_search_scope_label(sidebar, ps);

    // Spec 1E.4: query is primary text, date is secondary
    let query_style: fn(&iced::Theme) -> text::Style =
        if active { text::primary } else { text::base };
    let date_style: fn(&iced::Theme) -> text::Style = if active {
        text::secondary
    } else {
        theme::TextClass::Muted.style()
    };

    let mut meta_row = row![text(date_label).size(TEXT_SM).style(date_style),]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center);

    meta_row = meta_row.push(
        text("•")
            .size(TEXT_XS)
            .style(theme::TextClass::Muted.style()),
    );
    meta_row = meta_row.push(text(scope_label).size(TEXT_SM).style(date_style));

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
            icon::x()
                .size(ICON_XS)
                .style(theme::TextClass::Muted.style()),
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

    button(container(content).padding(PAD_NAV_ITEM))
        .on_press(SidebarMessage::SelectPinnedSearch(ps.id))
        .padding(0)
        .style(theme::ButtonClass::PinnedSearch { active }.style())
        .width(Length::Fill)
        .into()
}

fn pinned_search_scope_label(sidebar: &Sidebar, ps: &PinnedSearch) -> String {
    let Some(account_id) = ps.scope_account_id.as_deref() else {
        return "All Accounts".to_string();
    };

    sidebar
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .map(|account| {
            account
                .account_name
                .clone()
                .or_else(|| account.display_name.clone())
                .unwrap_or_else(|| account.email.clone())
        })
        .unwrap_or_else(|| "All Accounts".to_string())
}

/// Formats a unix timestamp as a relative time string (e.g. "5 min ago", "2 hours ago").
pub(crate) fn format_relative_time(timestamp: i64) -> String {
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
