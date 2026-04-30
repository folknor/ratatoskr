use crate::ui::undoable::UndoableText;

/// Which field in the contact editor changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactField {
    DisplayName,
    Email,
    Email2,
    Phone,
    Company,
    Notes,
}

/// Editing state for the contact editor sheet.
#[derive(Debug, Clone)]
pub struct ContactEditorState {
    pub contact_id: Option<String>,
    pub account_id: Option<String>,
    pub display_name: UndoableText,
    pub email: UndoableText,
    pub email2: UndoableText,
    pub phone: UndoableText,
    pub company: UndoableText,
    pub notes: UndoableText,
    /// Contact source: "user" (local), "google", "graph", "carddav", etc.
    /// None for newly created contacts (treated as local).
    pub source: Option<String>,
    /// Provider-assigned server ID for synced contacts.
    pub server_id: Option<String>,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

/// Editing state for the group editor sheet.
#[derive(Debug, Clone)]
pub struct GroupEditorState {
    pub group_id: Option<String>,
    pub name: UndoableText,
    pub members: Vec<String>,
    /// Filter text for the "Add Members" candidate list.
    pub filter: String,
    /// Filter text for the existing-members list at the bottom.
    pub members_filter: String,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

/// Re-export of `ContactField` from the import crate, used in settings messages.
/// We wrap it to derive the traits needed for iced messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportContactField {
    DisplayName,
    FirstName,
    LastName,
    Email,
    Email2,
    Phone,
    Company,
    Notes,
    Group,
    Ignore,
}

impl ImportContactField {
    pub const ALL_OPTIONS: &[ImportContactField] = &[
        ImportContactField::Ignore,
        ImportContactField::DisplayName,
        ImportContactField::FirstName,
        ImportContactField::LastName,
        ImportContactField::Email,
        ImportContactField::Email2,
        ImportContactField::Phone,
        ImportContactField::Company,
        ImportContactField::Notes,
        ImportContactField::Group,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ImportContactField::DisplayName => "Name",
            ImportContactField::FirstName => "First Name",
            ImportContactField::LastName => "Last Name",
            ImportContactField::Email => "Email",
            ImportContactField::Email2 => "Email 2",
            ImportContactField::Phone => "Phone",
            ImportContactField::Company => "Company",
            ImportContactField::Notes => "Notes",
            ImportContactField::Group => "Group",
            ImportContactField::Ignore => "Ignore",
        }
    }

    /// Convert to the import crate's `ContactField`.
    pub fn to_import_field(self) -> import::ContactField {
        match self {
            ImportContactField::DisplayName => import::ContactField::DisplayName,
            ImportContactField::FirstName => import::ContactField::FirstName,
            ImportContactField::LastName => import::ContactField::LastName,
            ImportContactField::Email => import::ContactField::Email,
            ImportContactField::Email2 => import::ContactField::Email2,
            ImportContactField::Phone => import::ContactField::Phone,
            ImportContactField::Company => import::ContactField::Company,
            ImportContactField::Notes => import::ContactField::Notes,
            ImportContactField::Group => import::ContactField::Group,
            ImportContactField::Ignore => import::ContactField::Ignore,
        }
    }

    /// Convert from the import crate's `ContactField`.
    pub fn from_import_field(field: import::ContactField) -> Self {
        match field {
            import::ContactField::DisplayName => ImportContactField::DisplayName,
            import::ContactField::FirstName => ImportContactField::FirstName,
            import::ContactField::LastName => ImportContactField::LastName,
            import::ContactField::Email => ImportContactField::Email,
            import::ContactField::Email2 => ImportContactField::Email2,
            import::ContactField::Phone => ImportContactField::Phone,
            import::ContactField::Company => ImportContactField::Company,
            import::ContactField::Notes => ImportContactField::Notes,
            import::ContactField::Group => ImportContactField::Group,
            import::ContactField::Ignore => ImportContactField::Ignore,
        }
    }
}

impl std::fmt::Display for ImportContactField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
