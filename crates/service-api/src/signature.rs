//! Signature CRUD wire types (Phase 6a).
//!
//! Four IPC methods - `signature.create`, `signature.update`,
//! `signature.delete`, `signature.reorder` - establish the CRUD shape
//! that contacts/groups will copy. Each gets its own params + ack
//! struct so a future change to one method's wire contract cannot
//! accidentally affect the others.
//!
//! `body_text` on the wire is `Option<String>`: `None` means "no
//! change" (update) or "compute from body_html on the Service side"
//! (create). Today's UI always supplies a body_text alongside any
//! body_html change, so the wire shape does not carry the
//! `Option<Option<String>>` "set to NULL" capability the underlying
//! DB function exposes - the boundary is the right place to enforce
//! that constraint.

use serde::{Deserialize, Serialize};

/// `signature.create` request body.
///
/// `body_text` is optional because today's UI always derives it from
/// `body_html`; the Service handler does the strip-HTML conversion
/// when `None` so callers can omit it. `is_default` / `is_reply_default`
/// being `true` clears the same flag on every other signature for the
/// account inside the same DB transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureCreateParams {
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
}

/// `signature.create` ack. Carries the new signature id so a future
/// caller can reference it without re-listing first - today's UI
/// re-lists via `db_get_all_signatures` after every CRUD, so the id is
/// not strictly required for re-render but is cheap to surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureCreateAck {
    pub id: String,
}

/// `signature.update` request body.
///
/// Each optional field is "no change" if absent / `null`, else "set to
/// this value." Setting `is_default` to `true` clears the same flag on
/// every other signature for the same account inside the DB
/// transaction (mirrors the create-time behavior).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_reply_default: Option<bool>,
}

/// `signature.update` ack. Empty struct; failure surfaces through
/// `ServiceResponse::Error`. The handler runs a single transaction so
/// an Ok ack implies all set fields are committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureUpdateAck;

/// `signature.delete` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureDeleteParams {
    pub id: String,
}

/// `signature.delete` ack. Empty struct - delete-of-missing is not an
/// error (the DELETE is idempotent), so the only failure mode the UI
/// sees is wire / DB transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureDeleteAck;

/// `signature.reorder` request body. `ordered_ids` is a flat list of
/// signature ids in their new display order; each id receives
/// `sort_order = index_in_list`. Ids absent from the list are not
/// touched (keeps prior `sort_order`), so callers who reorder a
/// per-account subset only need to pass that account's ids.
///
/// Per-account ordering hazard: rapid reorder clicks can land out of
/// order at the Service if the blocking pool is not order-preserving.
/// The dispatch arm tolerates the staleness today (next reload picks
/// up the canonical order); a generation-token wrapper is the
/// documented escape hatch if a real bug shows up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureReorderParams {
    pub ordered_ids: Vec<String>,
}

/// `signature.reorder` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureReorderAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_create_params_round_trip_through_serde() {
        let original = SignatureCreateParams {
            account_id: "acc-1".to_string(),
            name: "Work".to_string(),
            body_html: "<p>Best,<br>Atle</p>".to_string(),
            body_text: Some("Best,\nAtle".to_string()),
            is_default: true,
            is_reply_default: false,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SignatureCreateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn signature_create_params_skip_none_body_text() {
        let p = SignatureCreateParams {
            account_id: "acc-1".to_string(),
            name: "Work".to_string(),
            body_html: "<p>x</p>".to_string(),
            body_text: None,
            is_default: false,
            is_reply_default: false,
        };
        let json = serde_json::to_value(&p).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(
            !obj.contains_key("body_text"),
            "body_text=None must be omitted on the wire so the Service \
             can derive it from body_html"
        );
    }

    #[test]
    fn signature_create_ack_round_trips() {
        let original = SignatureCreateAck {
            id: "sig-uuid-1".to_string(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SignatureCreateAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn signature_update_params_round_trip_partial() {
        // Only is_default changes; other fields remain at None.
        let original = SignatureUpdateParams {
            id: "sig-1".to_string(),
            name: None,
            body_html: None,
            body_text: None,
            is_default: Some(true),
            is_reply_default: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(!obj.contains_key("name"));
        assert!(!obj.contains_key("body_html"));
        assert!(!obj.contains_key("body_text"));
        assert!(obj.contains_key("is_default"));
        assert!(!obj.contains_key("is_reply_default"));
        let recovered: SignatureUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn signature_update_params_round_trip_all_fields() {
        let original = SignatureUpdateParams {
            id: "sig-1".to_string(),
            name: Some("Work".to_string()),
            body_html: Some("<p>x</p>".to_string()),
            body_text: Some("x".to_string()),
            is_default: Some(false),
            is_reply_default: Some(true),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SignatureUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn signature_delete_params_round_trip() {
        let original = SignatureDeleteParams {
            id: "sig-1".to_string(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SignatureDeleteParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn signature_reorder_params_round_trip() {
        let original = SignatureReorderParams {
            ordered_ids: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: SignatureReorderParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
