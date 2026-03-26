use ratatoskr_db::db::types::AccountScope;

/// What the user is currently viewing — drives query routing and UI context.
///
/// This is a UI-scoping concept, not a database type. The translation from
/// `ViewScope` to `AccountScope` (for personal-account variants) happens in the
/// routing layer. `SharedMailbox` and `PublicFolder` scopes route to entirely
/// different query paths.
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
    /// Convert to `AccountScope` for query functions that only understand
    /// personal accounts. Returns `None` for scopes that need different
    /// query paths (shared mailbox, public folder).
    pub fn to_account_scope(&self) -> Option<AccountScope> {
        match self {
            Self::AllAccounts => Some(AccountScope::All),
            Self::Account(id) => Some(AccountScope::Single(id.clone())),
            Self::SharedMailbox { .. } | Self::PublicFolder { .. } => None,
        }
    }

    /// The parent account ID, if this scope is tied to a specific account.
    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::AllAccounts => None,
            Self::Account(id) => Some(id),
            Self::SharedMailbox { account_id, .. }
            | Self::PublicFolder { account_id, .. } => Some(account_id),
        }
    }
}
