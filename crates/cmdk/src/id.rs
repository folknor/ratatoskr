use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandId {
    // Navigation
    NavNext,
    NavPrev,
    NavOpen,
    NavMsgNext,
    NavMsgPrev,
    NavGoInbox,
    NavGoStarred,
    NavGoSent,
    NavGoDrafts,
    NavGoSnoozed,
    NavGoTrash,
    NavGoAllMail,
    NavGoPrimary,
    NavGoUpdates,
    NavGoPromotions,
    NavGoSocial,
    NavGoNewsletters,
    NavGoTasks,
    NavGoAttachments,
    NavEscape,
    NavigateToLabel,

    // Email
    EmailArchive,
    EmailTrash,
    EmailPermanentDelete,
    EmailSpam,
    EmailMarkRead,
    EmailStar,
    EmailPin,
    EmailMute,
    EmailUnsubscribe,
    EmailMoveToFolder,
    EmailAddLabel,
    EmailRemoveLabel,
    EmailSnooze,
    EmailSelectAll,
    EmailSelectFromHere,

    // Compose
    ComposeNew,
    ComposeReply,
    ComposeReplyAll,
    ComposeForward,

    // Tasks
    TaskCreate,
    TaskCreateFromEmail,
    TaskTogglePanel,
    TaskViewAll,

    // View
    ViewToggleSidebar,
    ViewSetThemeLight,
    ViewSetThemeDark,
    ViewSetThemeSystem,
    ViewToggleTaskPanel,
    ViewReadingPaneRight,
    ViewReadingPaneBottom,
    ViewReadingPaneHidden,

    // Calendar
    CalendarToggle,
    SwitchToCalendar,
    SwitchToMail,
    CalendarViewDay,
    CalendarViewWorkWeek,
    CalendarViewWeek,
    CalendarViewMonth,
    CalendarToday,
    CalendarCreateEvent,
    CalendarPopOut,

    // App
    AppSearch,
    AppAskAi,
    AppHelp,
    AppSyncFolder,
    AppOpenPalette,

    // Undo
    Undo,

    // Smart Folders
    SmartFolderSave,
}

const TABLE: &[(CommandId, &str)] = &[
    (CommandId::NavNext, "nav.next"),
    (CommandId::NavPrev, "nav.prev"),
    (CommandId::NavOpen, "nav.open"),
    (CommandId::NavMsgNext, "nav.msgNext"),
    (CommandId::NavMsgPrev, "nav.msgPrev"),
    (CommandId::NavGoInbox, "nav.goInbox"),
    (CommandId::NavGoStarred, "nav.goStarred"),
    (CommandId::NavGoSent, "nav.goSent"),
    (CommandId::NavGoDrafts, "nav.goDrafts"),
    (CommandId::NavGoSnoozed, "nav.goSnoozed"),
    (CommandId::NavGoTrash, "nav.goTrash"),
    (CommandId::NavGoAllMail, "nav.goAllMail"),
    (CommandId::NavGoPrimary, "nav.goPrimary"),
    (CommandId::NavGoUpdates, "nav.goUpdates"),
    (CommandId::NavGoPromotions, "nav.goPromotions"),
    (CommandId::NavGoSocial, "nav.goSocial"),
    (CommandId::NavGoNewsletters, "nav.goNewsletters"),
    (CommandId::NavGoTasks, "nav.goTasks"),
    (CommandId::NavGoAttachments, "nav.goAttachments"),
    (CommandId::NavEscape, "nav.escape"),
    (CommandId::NavigateToLabel, "nav.goLabel"),
    (CommandId::EmailArchive, "email.archive"),
    (CommandId::EmailTrash, "email.trash"),
    (CommandId::EmailPermanentDelete, "email.permanentDelete"),
    (CommandId::EmailSpam, "email.spam"),
    (CommandId::EmailMarkRead, "email.markRead"),
    (CommandId::EmailStar, "email.star"),
    (CommandId::EmailPin, "email.pin"),
    (CommandId::EmailMute, "email.mute"),
    (CommandId::EmailUnsubscribe, "email.unsubscribe"),
    (CommandId::EmailMoveToFolder, "email.moveToFolder"),
    (CommandId::EmailAddLabel, "email.addLabel"),
    (CommandId::EmailRemoveLabel, "email.removeLabel"),
    (CommandId::EmailSnooze, "email.snooze"),
    (CommandId::EmailSelectAll, "email.selectAll"),
    (CommandId::EmailSelectFromHere, "email.selectFromHere"),
    (CommandId::ComposeNew, "compose.new"),
    (CommandId::ComposeReply, "compose.reply"),
    (CommandId::ComposeReplyAll, "compose.replyAll"),
    (CommandId::ComposeForward, "compose.forward"),
    (CommandId::TaskCreate, "task.create"),
    (CommandId::TaskCreateFromEmail, "task.createFromEmail"),
    (CommandId::TaskTogglePanel, "task.togglePanel"),
    (CommandId::TaskViewAll, "task.viewAll"),
    (CommandId::ViewToggleSidebar, "view.toggleSidebar"),
    (CommandId::ViewSetThemeLight, "view.setThemeLight"),
    (CommandId::ViewSetThemeDark, "view.setThemeDark"),
    (CommandId::ViewSetThemeSystem, "view.setThemeSystem"),
    (CommandId::ViewToggleTaskPanel, "view.toggleTaskPanel"),
    (CommandId::ViewReadingPaneRight, "view.readingPaneRight"),
    (CommandId::ViewReadingPaneBottom, "view.readingPaneBottom"),
    (CommandId::ViewReadingPaneHidden, "view.readingPaneHidden"),
    (CommandId::CalendarToggle, "calendar.toggle"),
    (CommandId::CalendarViewDay, "calendar.viewDay"),
    (CommandId::CalendarViewWorkWeek, "calendar.viewWorkWeek"),
    (CommandId::CalendarViewWeek, "calendar.viewWeek"),
    (CommandId::CalendarViewMonth, "calendar.viewMonth"),
    (CommandId::CalendarToday, "calendar.today"),
    (CommandId::CalendarCreateEvent, "calendar.createEvent"),
    (CommandId::CalendarPopOut, "calendar.popOut"),
    (CommandId::SwitchToCalendar, "calendar.switchTo"),
    (CommandId::SwitchToMail, "app.switchToMail"),
    (CommandId::AppSearch, "app.search"),
    (CommandId::AppAskAi, "app.askAi"),
    (CommandId::AppHelp, "app.help"),
    (CommandId::AppSyncFolder, "app.syncFolder"),
    (CommandId::AppOpenPalette, "app.openPalette"),
    (CommandId::Undo, "app.undo"),
    (CommandId::SmartFolderSave, "smartFolder.save"),
];

impl CommandId {
    /// Returns the canonical stable string identifier for this command.
    ///
    /// This is the persistence and IPC format - keybinding overrides, frontend
    /// references, and serialized settings depend on these values. Do not change
    /// existing entries without an explicit data migration. Use `parse()` as the
    /// inverse.
    pub fn as_str(self) -> &'static str {
        TABLE
            .iter()
            .find(|(id, _)| *id == self)
            .map_or("unknown", |(_, s)| s)
    }

    pub fn parse(value: &str) -> Option<Self> {
        TABLE.iter().find(|(_, s)| *s == value).map(|(id, _)| *id)
    }

    pub fn all() -> &'static [CommandId] {
        ALL_IDS
    }
}

const ALL_IDS: &[CommandId] = &[
    CommandId::NavNext,
    CommandId::NavPrev,
    CommandId::NavOpen,
    CommandId::NavMsgNext,
    CommandId::NavMsgPrev,
    CommandId::NavGoInbox,
    CommandId::NavGoStarred,
    CommandId::NavGoSent,
    CommandId::NavGoDrafts,
    CommandId::NavGoSnoozed,
    CommandId::NavGoTrash,
    CommandId::NavGoAllMail,
    CommandId::NavGoPrimary,
    CommandId::NavGoUpdates,
    CommandId::NavGoPromotions,
    CommandId::NavGoSocial,
    CommandId::NavGoNewsletters,
    CommandId::NavGoTasks,
    CommandId::NavGoAttachments,
    CommandId::NavEscape,
    CommandId::NavigateToLabel,
    CommandId::EmailArchive,
    CommandId::EmailTrash,
    CommandId::EmailPermanentDelete,
    CommandId::EmailSpam,
    CommandId::EmailMarkRead,
    CommandId::EmailStar,
    CommandId::EmailPin,
    CommandId::EmailMute,
    CommandId::EmailUnsubscribe,
    CommandId::EmailMoveToFolder,
    CommandId::EmailAddLabel,
    CommandId::EmailRemoveLabel,
    CommandId::EmailSnooze,
    CommandId::EmailSelectAll,
    CommandId::EmailSelectFromHere,
    CommandId::ComposeNew,
    CommandId::ComposeReply,
    CommandId::ComposeReplyAll,
    CommandId::ComposeForward,
    CommandId::TaskCreate,
    CommandId::TaskCreateFromEmail,
    CommandId::TaskTogglePanel,
    CommandId::TaskViewAll,
    CommandId::ViewToggleSidebar,
    CommandId::ViewSetThemeLight,
    CommandId::ViewSetThemeDark,
    CommandId::ViewSetThemeSystem,
    CommandId::ViewToggleTaskPanel,
    CommandId::ViewReadingPaneRight,
    CommandId::ViewReadingPaneBottom,
    CommandId::ViewReadingPaneHidden,
    CommandId::CalendarToggle,
    CommandId::CalendarViewDay,
    CommandId::CalendarViewWorkWeek,
    CommandId::CalendarViewWeek,
    CommandId::CalendarViewMonth,
    CommandId::CalendarToday,
    CommandId::CalendarCreateEvent,
    CommandId::CalendarPopOut,
    CommandId::SwitchToCalendar,
    CommandId::SwitchToMail,
    CommandId::AppSearch,
    CommandId::AppAskAi,
    CommandId::AppHelp,
    CommandId::AppSyncFolder,
    CommandId::AppOpenPalette,
    CommandId::Undo,
    CommandId::SmartFolderSave,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_all_variants() {
        assert_eq!(TABLE.len(), ALL_IDS.len());
        for id in ALL_IDS {
            assert_ne!(id.as_str(), "unknown", "missing table entry for {id:?}");
        }
    }

    #[test]
    fn round_trip() {
        for id in ALL_IDS {
            let s = id.as_str();
            let parsed = CommandId::parse(s);
            assert_eq!(parsed, Some(*id), "round-trip failed for {s}");
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(CommandId::parse("nonexistent.command"), None);
    }
}
