/// What the user is currently viewing - drives query routing and UI context.
///
/// This is a UI-scoping concept, not a database type. Personal accounts,
/// shared mailboxes, and public folders route through different query paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    /// All personal accounts.
    AllAccounts,
    /// Single personal account by ID.
    Account(String),
    /// Shared mailbox, identified by parent account + mailbox address.
    SharedMailbox {
        account_id: String,
        mailbox_id: String,
    },
    /// Pinned public folder, identified by parent account + folder ID.
    PublicFolder {
        account_id: String,
        folder_id: String,
    },
}

impl ViewScope {
    /// The parent account ID, if this scope is tied to a specific account.
    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::AllAccounts => None,
            Self::Account(id) => Some(id),
            Self::SharedMailbox { account_id, .. } | Self::PublicFolder { account_id, .. } => {
                Some(account_id)
            }
        }
    }
}
