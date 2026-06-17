//! Smart-folder write surface (Phase 6a-part-2).
//!
//! One IPC method - `smart_folder.create` - relocates the
//! "save current search as smart folder" path Service-side. The UI
//! today calls `Db::create_smart_folder(name, query)` from
//! `handle_save_as_smart_folder`; that method mints a UUID UI-side and
//! discards it (returns `Ok(0)`). Post-relocation the Service mints
//! the UUID and returns it in the ack so future tightening (e.g. an
//! "open in" follow-up that needs the id) can plug in without a
//! second round-trip.
//!
//! Sibling table to `pinned_searches`. The two surfaces ship in
//! separate modules at the wire level even though they live in the
//! same UI module today (`db/pinned_searches.rs`); future work to
//! split the UI module is out of scope.
//!
//! Out of scope: smart-folder update / delete / icon-color edit.
//! These are not user-facing surfaces today (the UI offers no edit
//! affordance for smart folders), so 6a-part-2 only relocates the
//! single create path that exists.

use serde::{Deserialize, Serialize};

/// `smart_folder.create` request body. The UI passes `name` and
/// `query`; the Service mints the row id (UUID String) and stores
/// the row with default icon (`"search"`) and no color / no
/// account scope, matching today's UI-side defaults at
/// `app/src/db/pinned_searches.rs:122`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartFolderCreateParams {
    pub name: String,
    pub query: String,
}

/// `smart_folder.create` ack. Carries the minted UUID so callers can
/// reference the new row without re-listing first - today's UI does
/// re-list (sidebar navigation reload) so the id is informational
/// for now, but the wire shape avoids a future second round-trip if
/// a caller wants to navigate-on-create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartFolderCreateAck {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_params_round_trip_through_serde() {
        let original = SmartFolderCreateParams {
            name: "Unread VIPs".into(),
            query: "is:unread from:vip@example.com".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SmartFolderCreateParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn create_ack_round_trips() {
        let original = SmartFolderCreateAck {
            id: "11111111-2222-3333-4444-555555555555".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SmartFolderCreateAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
