use serde::{Deserialize, Serialize};

use crate::MailProviderKind;

const KEYWORD_PREFIX: &str = "kw:";
const CATEGORY_PREFIX: &str = "cat:";
const IMPORTANCE_HIGH_ID: &str = "importance:high";
const IMPORTANCE_LOW_ID: &str = "importance:low";
const GRAPH_FOLDER_PREFIX: &str = "graph-";
const JMAP_FOLDER_PREFIX: &str = "jmap-";
const IMAP_FOLDER_PREFIX: &str = "folder-";

/// Validated IMAP/JMAP user keyword payload.
///
/// ```compile_fail
/// use types::KeywordName;
/// let _ = KeywordName("todo".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeywordName(String);

/// Validated Exchange category payload.
///
/// ```compile_fail
/// use types::CategoryName;
/// let _ = CategoryName("Blue".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CategoryName(String);

/// Validated Microsoft Graph opaque folder ID.
///
/// ```compile_fail
/// use types::GraphGuid;
/// let _ = GraphGuid("AAMkAD…".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphGuid(String);

/// Validated IMAP folder path.
///
/// ```compile_fail
/// use types::ImapPath;
/// let _ = ImapPath("INBOX/Work".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImapPath(String);

/// Validated Gmail user-label ID.
///
/// ```compile_fail
/// use types::GmailLabelId;
/// let _ = GmailLabelId("Label_1".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GmailLabelId(String);

/// Validated Gmail non-role system folder label ID (`CHAT`, `CATEGORY_*`).
///
/// ```compile_fail
/// use types::GmailSystemLabelId;
/// let _ = GmailSystemLabelId("CHAT".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GmailSystemLabelId(String);

/// Validated JMAP mailbox ID.
///
/// ```compile_fail
/// use types::JmapId;
/// let _ = JmapId("Mb1".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JmapId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SystemFolderId {
    Inbox,
    Sent,
    Draft,
    Trash,
    Spam,
    Archive,
    Important,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FolderKind {
    System(SystemFolderId),
    GmailSystem(GmailSystemLabelId),
    GraphUser(GraphGuid),
    JmapUser(JmapId),
    ImapUser(ImapPath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImportanceLevel {
    High,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LabelKind {
    GmailUser(GmailLabelId),
    GraphCategory(CategoryName),
    GraphImportance(ImportanceLevel),
    JmapKeyword(KeywordName),
    ImapKeyword(KeywordName),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MailLocator {
    Folder(FolderKind),
    Label(LabelKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    FromFolders,
    FromLabels,
    FromUserQuery,
}

impl KeywordName {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("keyword", raw)?;
        if raw.starts_with('$') || is_reserved_imap_keyword(raw) {
            return Err(format!("reserved keyword `{raw}` is not a user label"));
        }
        if raw.starts_with(KEYWORD_PREFIX) {
            return Err(format!(
                "keyword body `{raw}` already carries the `{KEYWORD_PREFIX}` storage prefix"
            ));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn storage_id(&self) -> String {
        format!("{KEYWORD_PREFIX}{}", self.0)
    }
}

impl CategoryName {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("category", raw)?;
        if raw.starts_with(CATEGORY_PREFIX) {
            return Err(format!(
                "category body `{raw}` already carries the `{CATEGORY_PREFIX}` storage prefix"
            ));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn storage_id(&self) -> String {
        format!("{CATEGORY_PREFIX}{}", self.0)
    }
}

impl GraphGuid {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("graph folder id", raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn parse_storage_id(raw: &str) -> Result<Self, String> {
        let graph_id = raw
            .strip_prefix(GRAPH_FOLDER_PREFIX)
            .ok_or_else(|| format!("Graph user folder id `{raw}` must start with graph-"))?;
        Self::parse(graph_id)
    }

    pub fn from_graph_id(raw: &str) -> Result<Self, String> {
        Self::parse(raw)
    }

    pub fn as_graph_id(&self) -> &str {
        &self.0
    }

    fn storage_id(&self) -> String {
        format!("{GRAPH_FOLDER_PREFIX}{}", self.0)
    }
}

impl ImapPath {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("imap folder path", raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn parse_storage_id(raw: &str) -> Result<Self, String> {
        let path = raw
            .strip_prefix(IMAP_FOLDER_PREFIX)
            .ok_or_else(|| format!("IMAP user folder id `{raw}` must start with folder-"))?;
        Self::parse(path)
    }

    pub fn from_path(raw: &str) -> Result<Self, String> {
        Self::parse(raw)
    }

    pub fn as_path(&self) -> &str {
        &self.0
    }

    fn storage_id(&self) -> String {
        format!("{IMAP_FOLDER_PREFIX}{}", self.0)
    }
}

impl GmailLabelId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("gmail label id", raw)?;
        if SystemFolderId::parse(raw).is_some()
            || raw == "CHAT"
            || raw.starts_with("CATEGORY_")
            || raw.starts_with(KEYWORD_PREFIX)
            || raw.starts_with(CATEGORY_PREFIX)
            || raw.starts_with("importance:")
        {
            return Err(format!("Gmail label id `{raw}` is reserved for another kind"));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl GmailSystemLabelId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("gmail system label id", raw)?;
        if raw == "CHAT" || raw.starts_with("CATEGORY_") {
            return Ok(Self(raw.to_string()));
        }
        Err(format!(
            "Gmail system folder id `{raw}` is not a non-role system folder"
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JmapId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        validate_component("jmap mailbox id", raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn parse_storage_id(raw: &str) -> Result<Self, String> {
        let id = raw
            .strip_prefix(JMAP_FOLDER_PREFIX)
            .ok_or_else(|| format!("JMAP user folder id `{raw}` must start with jmap-"))?;
        Self::parse(id)
    }

    pub fn from_jmap_id(raw: &str) -> Result<Self, String> {
        Self::parse(raw)
    }

    pub fn as_jmap_id(&self) -> &str {
        &self.0
    }

    fn storage_id(&self) -> String {
        format!("{JMAP_FOLDER_PREFIX}{}", self.0)
    }
}

impl SystemFolderId {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "INBOX" => Some(Self::Inbox),
            "SENT" => Some(Self::Sent),
            "DRAFT" => Some(Self::Draft),
            "TRASH" => Some(Self::Trash),
            "SPAM" => Some(Self::Spam),
            "archive" => Some(Self::Archive),
            "IMPORTANT" => Some(Self::Important),
            _ => None,
        }
    }

    /// Parse a user-facing `in:` shorthand into the corresponding system folder.
    /// `Important` is intentionally absent - it's a Gmail-only concept and is
    /// exposed through `is:starred` and per-account label rendering, not `in:`.
    pub fn parse_shorthand(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "inbox" => Some(Self::Inbox),
            "sent" => Some(Self::Sent),
            "drafts" | "draft" => Some(Self::Draft),
            "trash" => Some(Self::Trash),
            "spam" => Some(Self::Spam),
            "archive" => Some(Self::Archive),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "INBOX",
            Self::Sent => "SENT",
            Self::Draft => "DRAFT",
            Self::Trash => "TRASH",
            Self::Spam => "SPAM",
            Self::Archive => "archive",
            Self::Important => "IMPORTANT",
        }
    }
}

impl FolderKind {
    pub fn parse(raw: &str, provider: MailProviderKind) -> Result<Self, String> {
        if let Some(system) = SystemFolderId::parse(raw) {
            return Ok(Self::System(system));
        }

        match provider {
            MailProviderKind::Gmail => GmailSystemLabelId::parse(raw).map(Self::GmailSystem),
            MailProviderKind::Graph => GraphGuid::parse_storage_id(raw).map(Self::GraphUser),
            MailProviderKind::Jmap => JmapId::parse_storage_id(raw).map(Self::JmapUser),
            MailProviderKind::Imap => ImapPath::parse_storage_id(raw).map(Self::ImapUser),
        }
    }

    pub fn gmail_system(raw: &str) -> Result<Self, String> {
        GmailSystemLabelId::parse(raw).map(Self::GmailSystem)
    }

    pub fn graph_user(raw_graph_id: &str) -> Result<Self, String> {
        GraphGuid::from_graph_id(raw_graph_id).map(Self::GraphUser)
    }

    pub fn jmap_user(raw_jmap_id: &str) -> Result<Self, String> {
        JmapId::from_jmap_id(raw_jmap_id).map(Self::JmapUser)
    }

    pub fn imap_user(path: &str) -> Result<Self, String> {
        ImapPath::from_path(path).map(Self::ImapUser)
    }

    pub fn storage_id(&self) -> String {
        match self {
            Self::System(system) => system.as_str().to_string(),
            Self::GmailSystem(id) => id.as_str().to_string(),
            Self::GraphUser(id) => id.storage_id(),
            Self::JmapUser(id) => id.storage_id(),
            Self::ImapUser(path) => path.storage_id(),
        }
    }
}

impl ImportanceLevel {
    pub const ALL: [Self; 2] = [Self::High, Self::Low];

    pub fn parse_label_id(raw: &str) -> Option<Self> {
        match raw {
            IMPORTANCE_HIGH_ID => Some(Self::High),
            IMPORTANCE_LOW_ID => Some(Self::Low),
            _ => None,
        }
    }

    pub fn from_graph_value(raw: &str) -> Option<Self> {
        if raw.eq_ignore_ascii_case("high") {
            Some(Self::High)
        } else if raw.eq_ignore_ascii_case("low") {
            Some(Self::Low)
        } else {
            None
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::High => Self::Low,
            Self::Low => Self::High,
        }
    }

    pub fn label_id(self) -> &'static str {
        match self {
            Self::High => IMPORTANCE_HIGH_ID,
            Self::Low => IMPORTANCE_LOW_ID,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::High => "High importance",
            Self::Low => "Low importance",
        }
    }

    pub fn sort_order(self) -> i64 {
        match self {
            Self::High => 10_000,
            Self::Low => 10_001,
        }
    }

    pub fn graph_value(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Low => "low",
        }
    }
}

impl LabelKind {
    pub fn parse(raw: &str, provider: MailProviderKind) -> Result<Self, String> {
        match provider {
            MailProviderKind::Gmail => GmailLabelId::parse(raw).map(Self::GmailUser),
            MailProviderKind::Graph => {
                if let Some(level) = ImportanceLevel::parse_label_id(raw) {
                    return Ok(Self::GraphImportance(level));
                }
                let category = raw
                    .strip_prefix(CATEGORY_PREFIX)
                    .ok_or_else(|| format!("Graph label id `{raw}` is not a category or importance label"))?;
                CategoryName::parse(category).map(Self::GraphCategory)
            }
            MailProviderKind::Jmap => {
                let keyword = raw
                    .strip_prefix(KEYWORD_PREFIX)
                    .ok_or_else(|| format!("JMAP label id `{raw}` is not a keyword label"))?;
                KeywordName::parse(keyword).map(Self::JmapKeyword)
            }
            MailProviderKind::Imap => {
                let keyword = raw
                    .strip_prefix(KEYWORD_PREFIX)
                    .ok_or_else(|| format!("IMAP label id `{raw}` is not a keyword label"))?;
                KeywordName::parse(keyword).map(Self::ImapKeyword)
            }
        }
    }

    pub fn gmail_user(raw: &str) -> Result<Self, String> {
        GmailLabelId::parse(raw).map(Self::GmailUser)
    }

    pub fn graph_category(raw: &str) -> Result<Self, String> {
        CategoryName::parse(raw).map(Self::GraphCategory)
    }

    pub fn graph_importance(level: ImportanceLevel) -> Self {
        Self::GraphImportance(level)
    }

    pub fn jmap_keyword(raw: &str) -> Result<Self, String> {
        KeywordName::parse(raw).map(Self::JmapKeyword)
    }

    pub fn imap_keyword(raw: &str) -> Result<Self, String> {
        KeywordName::parse(raw).map(Self::ImapKeyword)
    }

    pub fn storage_id(&self) -> String {
        match self {
            Self::GmailUser(id) => id.as_str().to_string(),
            Self::GraphCategory(category) => category.storage_id(),
            Self::GraphImportance(level) => level.label_id().to_string(),
            Self::JmapKeyword(keyword) | Self::ImapKeyword(keyword) => keyword.storage_id(),
        }
    }
}

impl MailLocator {
    pub fn parse(
        raw: &str,
        provider: MailProviderKind,
        namespace: Namespace,
    ) -> Result<Self, String> {
        match namespace {
            Namespace::FromFolders => FolderKind::parse(raw, provider).map(Self::Folder),
            Namespace::FromLabels => LabelKind::parse(raw, provider).map(Self::Label),
            Namespace::FromUserQuery => FolderKind::parse(raw, provider)
                .map(Self::Folder)
                .or_else(|_| LabelKind::parse(raw, provider).map(Self::Label)),
        }
    }
}

fn validate_component(kind: &str, raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err(format!("{kind} cannot be empty"));
    }
    if raw.chars().any(char::is_control) {
        return Err(format!("{kind} cannot contain control characters"));
    }
    Ok(())
}

fn is_reserved_imap_keyword(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "$forwarded" | "$mdnsent" | "$junk" | "$notjunk" | "$phishing"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_label_kinds_by_provider() {
        assert!(matches!(
            LabelKind::parse("Label_123", MailProviderKind::Gmail).unwrap(),
            LabelKind::GmailUser(_),
        ));
        assert!(matches!(
            LabelKind::parse("cat:Blue", MailProviderKind::Graph).unwrap(),
            LabelKind::GraphCategory(_),
        ));
        assert_eq!(
            LabelKind::parse("importance:high", MailProviderKind::Graph)
                .unwrap()
                .storage_id(),
            "importance:high",
        );
        assert!(matches!(
            LabelKind::parse("kw:todo", MailProviderKind::Jmap).unwrap(),
            LabelKind::JmapKeyword(_),
        ));
        assert!(matches!(
            LabelKind::parse("kw:todo", MailProviderKind::Imap).unwrap(),
            LabelKind::ImapKeyword(_),
        ));
    }

    #[test]
    fn rejects_cross_kind_labels() {
        assert!(LabelKind::parse("INBOX", MailProviderKind::Gmail).is_err());
        assert!(LabelKind::parse("importance:medium", MailProviderKind::Graph).is_err());
        assert!(LabelKind::parse("cat:Blue", MailProviderKind::Imap).is_err());
        assert!(LabelKind::parse("kw:$junk", MailProviderKind::Jmap).is_err());
    }

    #[test]
    fn parses_folder_kinds_by_provider() {
        assert_eq!(
            FolderKind::parse("INBOX", MailProviderKind::Gmail)
                .unwrap()
                .storage_id(),
            "INBOX",
        );
        assert!(matches!(
            FolderKind::parse("graph-abc", MailProviderKind::Graph).unwrap(),
            FolderKind::GraphUser(_),
        ));
        assert!(matches!(
            FolderKind::parse("jmap-mailbox", MailProviderKind::Jmap).unwrap(),
            FolderKind::JmapUser(_),
        ));
        assert!(matches!(
            FolderKind::parse("folder-Projects", MailProviderKind::Imap).unwrap(),
            FolderKind::ImapUser(_),
        ));
        assert!(matches!(
            FolderKind::parse("CATEGORY_PROMOTIONS", MailProviderKind::Gmail).unwrap(),
            FolderKind::GmailSystem(_),
        ));
        assert!(FolderKind::parse("STARRED", MailProviderKind::Gmail).is_err());
    }

    #[test]
    fn importance_round_trips_without_strings_at_callers() {
        let high = ImportanceLevel::parse_label_id("importance:high").unwrap();
        assert_eq!(high.opposite(), ImportanceLevel::Low);
        assert_eq!(high.graph_value(), "high");
        assert_eq!(ImportanceLevel::Low.label_id(), "importance:low");
    }

    #[test]
    fn system_folder_shorthands_are_exhaustive() {
        assert_eq!(
            SystemFolderId::parse_shorthand("drafts").unwrap().as_str(),
            "DRAFT",
        );
        assert_eq!(
            SystemFolderId::parse_shorthand("archive").unwrap().as_str(),
            "archive",
        );
        assert!(SystemFolderId::parse_shorthand("important").is_none());
    }

    #[test]
    fn label_kinds_storage_round_trip() {
        let cases = [
            (LabelKind::gmail_user("Label_1").unwrap(), MailProviderKind::Gmail),
            (LabelKind::graph_category("Blue").unwrap(), MailProviderKind::Graph),
            (LabelKind::graph_importance(ImportanceLevel::High), MailProviderKind::Graph),
            (LabelKind::jmap_keyword("todo").unwrap(), MailProviderKind::Jmap),
            (LabelKind::imap_keyword("todo").unwrap(), MailProviderKind::Imap),
        ];
        for (label, provider) in cases {
            let id = label.storage_id();
            let parsed = LabelKind::parse(&id, provider).unwrap();
            assert_eq!(parsed.storage_id(), id);
            assert_eq!(parsed, label);
        }
    }

    #[test]
    fn folder_kinds_storage_round_trip() {
        let cases = [
            (FolderKind::graph_user("AAMkAD").unwrap(), MailProviderKind::Graph),
            (FolderKind::jmap_user("Mb-1").unwrap(), MailProviderKind::Jmap),
            (FolderKind::imap_user("INBOX/Work").unwrap(), MailProviderKind::Imap),
            (FolderKind::System(SystemFolderId::Inbox), MailProviderKind::Imap),
            (FolderKind::System(SystemFolderId::Archive), MailProviderKind::Gmail),
            (
                FolderKind::gmail_system("CHAT").unwrap(),
                MailProviderKind::Gmail,
            ),
        ];
        for (folder, provider) in cases {
            let id = folder.storage_id();
            let parsed = FolderKind::parse(&id, provider).unwrap();
            assert_eq!(parsed.storage_id(), id);
            assert_eq!(parsed, folder);
        }
    }

    #[test]
    fn keyword_name_rejects_storage_prefix() {
        assert!(KeywordName::parse("kw:todo").is_err());
        assert!(LabelKind::imap_keyword("kw:todo").is_err());
        assert!(LabelKind::jmap_keyword("kw:todo").is_err());
    }

    #[test]
    fn category_name_rejects_storage_prefix() {
        assert!(CategoryName::parse("cat:Blue").is_err());
        assert!(LabelKind::graph_category("cat:Blue").is_err());
    }
}
