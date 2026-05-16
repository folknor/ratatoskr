use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Canonical mail account provider identity.
///
/// ```compile_fail,E0308
/// use types::MailProviderKind;
///
/// fn accepts_provider(_: MailProviderKind) {}
///
/// accepts_provider("gmail_api");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MailProviderKind {
    Gmail,
    Graph,
    Jmap,
    Imap,
}

impl MailProviderKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "gmail_api" => Ok(Self::Gmail),
            "graph" => Ok(Self::Graph),
            "jmap" => Ok(Self::Jmap),
            "imap" => Ok(Self::Imap),
            other => Err(format!("Unknown provider: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gmail => "gmail_api",
            Self::Graph => "graph",
            Self::Jmap => "jmap",
            Self::Imap => "imap",
        }
    }
}

impl Serialize for MailProviderKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MailProviderKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::MailProviderKind;

    #[test]
    fn parses_canonical_account_provider_values() {
        assert_eq!(
            MailProviderKind::parse("gmail_api").unwrap(),
            MailProviderKind::Gmail,
        );
        assert_eq!(
            MailProviderKind::parse("graph").unwrap(),
            MailProviderKind::Graph,
        );
        assert_eq!(
            MailProviderKind::parse("jmap").unwrap(),
            MailProviderKind::Jmap,
        );
        assert_eq!(
            MailProviderKind::parse("imap").unwrap(),
            MailProviderKind::Imap,
        );
    }

    #[test]
    fn rejects_unknown_provider_values() {
        let err = MailProviderKind::parse("microsoft_graph").unwrap_err();
        assert!(err.contains("Unknown provider"));
    }
}
