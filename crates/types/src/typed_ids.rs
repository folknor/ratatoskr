// Typed ID newtypes for provider API parameters.
//
// These prevent passing a folder ID where a label ID is expected (or vice
// versa) at compile time. All providers receive the correct type and can
// pattern-match or strip prefixes as needed.

/// A folder/container ID (provider folder or canonical system folder).
/// Used with `move_to_folder`, `rename_folder`, `delete_folder`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct FolderId(pub String);

/// A label ID (Gmail user label, JMAP keyword, Graph category, IMAP keyword).
/// Used with `add_label`, `remove_label`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct LabelId(pub String);

/// A user-created cross-account label group ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct LabelGroupId(pub i64);

impl FolderId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl LabelId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl LabelGroupId {
    pub fn as_i64(self) -> i64 {
        self.0
    }
}

impl AsRef<str> for FolderId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LabelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for FolderId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<String> for LabelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for FolderId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<&str> for LabelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<i64> for LabelGroupId {
    fn from(id: i64) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for FolderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for LabelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for LabelGroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
