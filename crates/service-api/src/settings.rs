//! Settings wire types.
//!
//! Phase 6a (`docs/service/phase-6a-plan.md`) relocates the global
//! `settings` key/value table writes Service-side. Today's only call
//! site is `handle_settings_event::PreferencesCommitted` in the app
//! crate, which writes seven settings in one atomic transaction.
//!
//! Wire shape preserves that atomicity: `SettingsSetParams` carries a
//! `Vec<SettingValue>` and the Service-side handler writes them all in
//! a single transaction. A partial commit on Service crash mid-write
//! is impossible by SQLite construction.
//!
//! `SettingValue` is a typed enum with one variant per persisted
//! setting. The exhaustive-match house style (mirrors `MailOperation`)
//! makes adding a setting a compile-time-visible change at every
//! dispatch site rather than a stringly-typed runtime check.

use serde::{Deserialize, Serialize};

/// One persisted setting. Each variant corresponds to one row in the
/// global `settings` table. Adding a new persisted setting adds a
/// variant here; the Service-side handler's exhaustive match then
/// fails to compile until the new variant is wired through.
///
/// Serializes as `{ "type": "show_sync_status", "value": true }` style
/// (serde tag/content). See the round-trip test for the wire shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SettingValue {
    ShowSyncStatus(bool),
    BlockRemoteImages(bool),
    PhishingDetectionEnabled(bool),
    PhishingSensitivity(String),
    Theme(String),
    FontSize(String),
    ReadingPanePosition(String),
}

impl SettingValue {
    /// Storage key for the row in the `settings` table.
    pub fn key(&self) -> &'static str {
        match self {
            Self::ShowSyncStatus(_) => "show_sync_status",
            Self::BlockRemoteImages(_) => "block_remote_images",
            Self::PhishingDetectionEnabled(_) => "phishing_detection_enabled",
            Self::PhishingSensitivity(_) => "phishing_sensitivity",
            Self::Theme(_) => "theme",
            Self::FontSize(_) => "font_size",
            Self::ReadingPanePosition(_) => "reading_pane_position",
        }
    }

    /// Render the value for the storage row's `TEXT` column.
    /// Booleans encode as `"true"` / `"false"` to match today's
    /// `set_setting` callers.
    pub fn render_for_storage(&self) -> String {
        match self {
            Self::ShowSyncStatus(v)
            | Self::BlockRemoteImages(v)
            | Self::PhishingDetectionEnabled(v) => {
                if *v {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Self::PhishingSensitivity(s)
            | Self::Theme(s)
            | Self::FontSize(s)
            | Self::ReadingPanePosition(s) => s.clone(),
        }
    }
}

/// `settings.set` request body. Carries one or more `SettingValue`s
/// that the Service writes in a single atomic transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSetParams {
    pub values: Vec<SettingValue>,
}

/// `settings.set` ack. Empty struct; failure surfaces through
/// `ServiceResponse::Error`. Transaction atomicity means a successful
/// ack implies all values are committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSetAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_value_round_trips_through_serde() {
        let cases = vec![
            SettingValue::ShowSyncStatus(true),
            SettingValue::BlockRemoteImages(false),
            SettingValue::Theme("dark".to_string()),
            SettingValue::FontSize("medium".to_string()),
            SettingValue::ReadingPanePosition("right".to_string()),
        ];
        for value in cases {
            let json = serde_json::to_value(&value).expect("serialize");
            let recovered: SettingValue = serde_json::from_value(json).expect("deserialize");
            assert_eq!(value, recovered);
        }
    }

    #[test]
    fn settings_set_params_round_trips() {
        let params = SettingsSetParams {
            values: vec![
                SettingValue::ShowSyncStatus(true),
                SettingValue::Theme("light".to_string()),
            ],
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: SettingsSetParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn setting_value_storage_keys_are_unique() {
        let values = [
            SettingValue::ShowSyncStatus(true),
            SettingValue::BlockRemoteImages(true),
            SettingValue::PhishingDetectionEnabled(true),
            SettingValue::PhishingSensitivity("medium".to_string()),
            SettingValue::Theme("light".to_string()),
            SettingValue::FontSize("medium".to_string()),
            SettingValue::ReadingPanePosition("right".to_string()),
        ];
        let mut keys: Vec<_> = values.iter().map(SettingValue::key).collect();
        keys.sort_unstable();
        let original_len = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), original_len, "setting keys must be unique");
    }

    #[test]
    fn bool_setting_renders_as_true_false_string() {
        assert_eq!(SettingValue::ShowSyncStatus(true).render_for_storage(), "true");
        assert_eq!(
            SettingValue::ShowSyncStatus(false).render_for_storage(),
            "false"
        );
    }

    #[test]
    fn settings_set_ack_round_trips() {
        let ack = SettingsSetAck;
        let json = serde_json::to_value(&ack).expect("serialize");
        let _recovered: SettingsSetAck = serde_json::from_value(json).expect("deserialize");
    }
}
