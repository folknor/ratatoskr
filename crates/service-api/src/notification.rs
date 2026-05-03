use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoalesceKey(pub String);

impl CoalesceKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationClass {
    Coalesce { key: CoalesceKey },
    Drop,
    MustDeliver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// Test-only variant. Lets the wire round-trip be exercised before any
    /// production notifications exist. Compiled out of release builds via
    /// `#[cfg(test)]`.
    #[cfg(test)]
    #[serde(rename = "test.echo")]
    TestEcho { value: String },
}

impl Notification {
    pub fn class(&self) -> NotificationClass {
        #[cfg(not(test))]
        {
            match *self {}
        }
        #[cfg(test)]
        match self {
            Self::TestEcho { .. } => NotificationClass::Coalesce {
                key: CoalesceKey::new("test.echo"),
            },
        }
    }

    pub fn method_name(&self) -> &'static str {
        #[cfg(not(test))]
        {
            match *self {}
        }
        #[cfg(test)]
        match self {
            Self::TestEcho { .. } => "test.echo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{ParsedServiceMessage, parse_service_message};

    #[test]
    fn test_echo_round_trips_through_serde() {
        let original = Notification::TestEcho {
            value: "hello".to_string(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: Notification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_echo_round_trips_through_parse_service_message() {
        // The wire envelope is `{jsonrpc, method, params}`; parse_service_message
        // takes the line and reconstructs `Notification`. Verifies that the
        // synthetic JSON object the parser builds matches the
        // `tag = "method", content = "params"` shape that serde expects on the
        // way back in.
        let line = r#"{"jsonrpc":"2.0","method":"test.echo","params":{"value":"hi"}}"#;
        let parsed = parse_service_message(line).expect("parse");
        match parsed {
            ParsedServiceMessage::Notification(Notification::TestEcho { value }) => {
                assert_eq!(value, "hi");
            }
            other => panic!("expected TestEcho notification, got {other:?}"),
        }
    }

    #[test]
    fn test_echo_classifies_as_coalesce() {
        let notification = Notification::TestEcho {
            value: "x".to_string(),
        };
        match notification.class() {
            NotificationClass::Coalesce { key } => assert_eq!(key, CoalesceKey::new("test.echo")),
            other => panic!("expected Coalesce, got {other:?}"),
        }
    }

    #[test]
    fn test_echo_method_name_is_dotted() {
        let notification = Notification::TestEcho {
            value: "x".to_string(),
        };
        assert_eq!(notification.method_name(), "test.echo");
    }
}
