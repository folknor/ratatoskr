mod accounts;
mod calendar;
mod connection;
mod contacts;
mod palette;
mod pinned_searches;
pub mod threads;
mod types;

pub use connection::Db;
pub use contacts::{ContactEntry, ContactMatch, GroupEntry};
pub use pinned_searches::PinnedSearch;
pub use threads::{AppThreadDetail, ResolvedLabel};
pub use types::{
    Account, CalendarEvent, DateDisplay, MessageViewAttachment, PinnedPublicFolder, SharedMailbox,
    Thread, ThreadAttachment, ThreadMessage,
};
