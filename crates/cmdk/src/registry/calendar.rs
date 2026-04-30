use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::keybinding::KeyBinding;

use super::builders::{desc, desc_kw, with_docs};
use super::scoring::always;

pub(super) fn register_calendar(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc_kw(
            CommandId::CalendarToggle,
            "Toggle Calendar",
            "Calendar",
            Some(KeyBinding::cmd_or_ctrl('2')),
            always,
            &["switch mode", "mail", "calendar"],
        ),
        "Toggle Calendar",
        "Switch between mail and calendar views.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::SwitchToCalendar,
            "Calendar",
            "Calendar",
            None,
            always,
            &["open calendar", "show calendar"],
        ),
        "Switch to Calendar",
        "Switch to the calendar view.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::SwitchToMail,
            "Mail",
            "Mail",
            Some(KeyBinding::cmd_or_ctrl('1')),
            always,
            &["open mail", "show mail", "inbox"],
        ),
        "Switch to Mail",
        "Switch to the mail view.",
    ));
    out.push(with_docs(
        desc(CommandId::CalendarViewDay, "Day", "Calendar", None, always),
        "Day View",
        "Show a single day in the calendar.",
    ));
    out.push(with_docs(
        desc(
            CommandId::CalendarViewWorkWeek,
            "Work Week",
            "Calendar",
            None,
            always,
        ),
        "Work Week View",
        "Show Monday through Friday in the calendar.",
    ));
    out.push(with_docs(
        desc(
            CommandId::CalendarViewWeek,
            "Week",
            "Calendar",
            None,
            always,
        ),
        "Week View",
        "Show all seven days of the week in the calendar.",
    ));
    out.push(with_docs(
        desc(
            CommandId::CalendarViewMonth,
            "Month",
            "Calendar",
            None,
            always,
        ),
        "Month View",
        "Show the full month grid in the calendar.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::CalendarToday,
            "Today",
            "Calendar",
            None,
            always,
            &["today", "now", "current date"],
        ),
        "Go to Today",
        "Jump the calendar view to today's date.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::CalendarCreateEvent,
            "New Event",
            "Calendar",
            None,
            always,
            &["new event", "add event", "create"],
        ),
        "Create Event",
        "Open the event creation dialog.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::CalendarPopOut,
            "Pop Out",
            "Calendar",
            None,
            always,
            &["separate window", "multi monitor", "detach calendar"],
        ),
        "Pop Out Calendar",
        "Open the calendar in a separate window for multi-monitor setups.",
    ));
}
