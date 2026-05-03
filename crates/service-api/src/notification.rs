use crate::boot::{BootPhaseKind, BootProgress};
use serde::{Deserialize, Serialize};

/// Per-class coalesce key. The queue compares `CoalesceKey` for equality
/// when deciding whether a new entry replaces an existing one; the type is
/// constructed at enqueue time, never serialized onto the wire (the wire
/// notification carries the typed payload instead).
///
/// Each production variant is a discrete coalesce bucket. `BootProgress`
/// keys off `BootPhaseKind` so each phase coalesces independently and the
/// ordered phase sequence reaches the UI even under back-pressure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CoalesceKey {
    BootProgress(BootPhaseKind),
    /// Synthetic key used only by in-process queue tests in consumer crates.
    /// The queue's per-class enqueue logic is generic over `Classifiable`,
    /// and tests construct mock items with arbitrary string-keyed coalesce
    /// keys. Production code paths never construct `Test`.
    Test(String),
}

impl CoalesceKey {
    /// Construct a `Test` coalesce key. Test-only - the constructor name
    /// reflects that.
    pub fn test(value: impl Into<String>) -> Self {
        Self::Test(value.into())
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
    /// Service boot progress. Emitted during the boot sequence so the UI
    /// splash can render phase + per-migration progress while migrations,
    /// pending-ops recovery, drafts sweep, and thread-participants backfill
    /// run.
    #[serde(rename = "boot.progress")]
    BootProgress(BootProgress),
    /// Test-only variant. Lets the wire round-trip be exercised when no
    /// production payload happens to match a test's needs. Compiled out of
    /// release builds via `#[cfg(test)]`.
    #[cfg(test)]
    #[serde(rename = "test.echo")]
    TestEcho { value: String },
}

impl Notification {
    pub fn class(&self) -> NotificationClass {
        match self {
            Self::BootProgress(progress) => NotificationClass::Coalesce {
                key: CoalesceKey::BootProgress(progress.phase.coalesce_discriminant()),
            },
            #[cfg(test)]
            Self::TestEcho { .. } => NotificationClass::Coalesce {
                key: CoalesceKey::test("test.echo"),
            },
        }
    }

    pub fn method_name(&self) -> &'static str {
        match self {
            Self::BootProgress(_) => "boot.progress",
            #[cfg(test)]
            Self::TestEcho { .. } => "test.echo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::BootPhase;
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
            NotificationClass::Coalesce { key } => assert_eq!(key, CoalesceKey::test("test.echo")),
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

    #[test]
    fn boot_progress_round_trips_through_parse_service_message() {
        let line = r#"{"jsonrpc":"2.0","method":"boot.progress","params":{"phase":{"Migrating":{"current":3,"total":10}},"service_generation":7}}"#;
        let parsed = parse_service_message(line).expect("parse");
        match parsed {
            ParsedServiceMessage::Notification(Notification::BootProgress(progress)) => {
                assert_eq!(
                    progress.phase,
                    BootPhase::Migrating {
                        current: 3,
                        total: 10,
                    },
                );
                assert_eq!(progress.message, None);
                assert_eq!(progress.service_generation, 7);
            }
            other => panic!("expected BootProgress notification, got {other:?}"),
        }
    }

    #[test]
    fn boot_progress_classifies_per_phase() {
        let migrating_a = Notification::BootProgress(BootProgress {
            phase: BootPhase::Migrating {
                current: 1,
                total: 10,
            },
            message: None,
            service_generation: 0,
        });
        let migrating_b = Notification::BootProgress(BootProgress {
            phase: BootPhase::Migrating {
                current: 5,
                total: 10,
            },
            message: None,
            service_generation: 0,
        });
        let loading_key = Notification::BootProgress(BootProgress {
            phase: BootPhase::LoadingKey,
            message: None,
            service_generation: 0,
        });

        // Two `Migrating` updates collapse onto the same key, so the queue
        // replaces the older one with the newer.
        assert_eq!(migrating_a.class(), migrating_b.class());

        // `LoadingKey` keys onto a distinct discriminant; queue keeps both
        // entries.
        assert_ne!(migrating_a.class(), loading_key.class());
    }

    #[test]
    fn boot_progress_method_name_is_dotted() {
        let progress = Notification::BootProgress(BootProgress {
            phase: BootPhase::OpeningDatabase,
            message: None,
            service_generation: 0,
        });
        assert_eq!(progress.method_name(), "boot.progress");
    }

    #[test]
    fn coalesce_key_round_trips_through_serde() {
        let key = CoalesceKey::BootProgress(BootPhaseKind::Migrating);
        let json = serde_json::to_value(&key).expect("serialize");
        let recovered: CoalesceKey = serde_json::from_value(json).expect("deserialize");
        assert_eq!(key, recovered);
    }
}
