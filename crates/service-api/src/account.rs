//! Account write surfaces (Phase 6a).
//!
//! Phase 6a relocates the small / non-envelope account writes
//! (`account.update`, `account.reorder`) that don't need
//! orchestration. The bigger surfaces - `account.create` (with the
//! `Plaintext | Encrypted` credential envelope) and `account.delete`
//! (with the runner-cancel-and-await orchestration) - land as their
//! own modules so the wire shape for the cancel/delete state machine
//! does not bleed into the simple update path.
//!
//! Credential encryption: `account.create` and `account.update_tokens`
//! encrypt at the Service handler boundary. UI ships
//! `AccountCredentials::Plaintext`; the handler routes through
//! `common::crypto::encrypt_value` using the key held in
//! `BootSharedState`. The `Encrypted` variant is kept for callers
//! that already hold ciphertext (today: tests; future: any path
//! where the UI receives a pre-encrypted credential blob).
//!
//! `caldav_password` on `account.update` is still passed through
//! verbatim (the column does not currently store ciphertext for the
//! caldav path); this is documented drift, tracked separately.

use serde::{Deserialize, Serialize};

use crate::redacted::RedactedString;

/// `account.update` request body. Each `Option` field is "no change"
/// if `None`, else "set to value." Mirrors the existing
/// `UpdateAccountParams` struct from `db::queries_extra::accounts_crud`,
/// scoped to fields the settings panel exposes today (account
/// metadata + caldav credentials). Provider tokens / mailbox
/// password are deliberately not on this surface - those mutate via
/// the account-create flow and the future
/// `internal.encrypt_for_storage` IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_password: Option<String>,
    /// Attachments roadmap Phase 6: per-account offline-cache master
    /// switch. `Some(false)` disables prefetch + post-sync sweep for
    /// this account; `Some(true)` re-enables and the next sync /
    /// boot recovery kick picks up the backlog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_attachments_enabled: Option<bool>,
}

/// `account.update` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateAck;

/// One `(account_id, sort_order)` reassignment for the batch reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderEntry {
    pub account_id: String,
    pub sort_order: i64,
}

/// `account.reorder` request body. Account ids absent from `orders`
/// keep their existing `sort_order` - same convention as
/// `signature.reorder`. Per-account ordering hazard is the same as
/// signature reorder: rapid drag-reorder clicks can land out of
/// order; today's tolerance is "next reload reconciles."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderParams {
    pub orders: Vec<AccountReorderEntry>,
}

/// `account.reorder` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderAck;

/// Credentials envelope for `account.create`.
///
/// Day-one wire shape so 6b's two-step OAuth flow can extend rather
/// than redefine. Variants:
///
/// - `Plaintext`: caller passes raw secrets. Service handler routes
///   through `common::crypto::encrypt_value` using the key held in
///   `BootSharedState` before the DB write. UI's normal account-add
///   flow ships this variant.
/// - `Encrypted`: caller passes already-encrypted blobs in
///   `enc:base64iv:base64ct` form. Used by paths where the UI
///   already holds the encrypted material (today: tests; future
///   recovery flows). Service handler passes through verbatim.
///
/// Phase 6b will add a third `Oauth { auth_code, redirect_uri,
/// code_verifier }` variant for the fresh-OAuth-grant flow rather
/// than redefine this shape - hence "two variants today, room for a
/// third later" wording in the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountCredentials {
    /// Caller passes raw secrets. Service handler encrypts at the
    /// boundary before writing.
    Plaintext {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        imap_password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        smtp_password: Option<String>,
    },
    /// Caller passes already-encrypted blobs. Service handler writes
    /// them verbatim. Used by callers that already hold ciphertext
    /// (today: tests; future: any path where the UI receives a
    /// pre-encrypted credential blob).
    Encrypted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        imap_password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        smtp_password: Option<String>,
    },
}

/// `account.create` request body. Mirrors today's `CreateAccountParams`
/// in `db::queries_extra::accounts_crud` field-for-field, with
/// secrets pulled out into the typed `AccountCredentials` envelope so
/// the wire can carry both plaintext and pre-encrypted forms without
/// branching at every call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCreateParams {
    pub email: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub account_name: String,
    /// Required by the underlying DB schema (NOT NULL with no
    /// default). The UI's color picker always supplies a value, so
    /// keeping this required at the wire level lets the type system
    /// enforce what the schema already does.
    pub account_color: String,
    pub auth_method: String,
    pub credentials: AccountCredentials,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    /// Confidential-client secret from RFC 7591 dynamic registration or
    /// from a user-supplied value in the Custom OIDC wizard. None for
    /// public clients (the common case).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret: Option<String>,
    /// Space-separated extra OAuth scopes appended to the negotiated
    /// set during the auth-code request. NULL when no extras.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_extra_scopes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_port: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_security: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_port: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_security: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jmap_url: Option<String>,
    pub accept_invalid_certs: bool,
}

/// `account.create` ack. Carries the new account id so the UI can
/// kick off post-create flows (initial sync, calendar sync) without
/// re-listing first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCreateAck {
    pub id: String,
}

/// `account.update_tokens` request body.
///
/// Phase 6a-part-2: re-authentication path. The UI re-issues an
/// OAuth flow (or a password prompt) and gets fresh tokens; this
/// IPC writes them onto the existing account row without touching
/// identity or provider columns. Token / password fields wrap as
/// `RedactedString` so the wire-debug log surface cannot leak
/// them. Password and OAuth callers populate disjoint subsets of
/// the optional fields (the underlying DB helper builds the SET
/// list dynamically); a caller passing both is technically valid
/// but not a current code path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateTokensParams {
    pub account_id: String,
    pub access_token: Option<RedactedString>,
    pub refresh_token: Option<RedactedString>,
    pub token_expires_at: Option<i64>,
    pub imap_password: Option<RedactedString>,
    pub smtp_password: Option<RedactedString>,
}

/// `account.update_tokens` ack. Empty struct - same shape as
/// `AccountUpdateAck`. Failure surfaces via `ServiceResponse::Error`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateTokensAck;

/// `account.delete` request body.
///
/// Phase 6a-part-2: the Service-side handler runs the full deletion
/// sequence inside one IPC. The sequence is cancel-and-await for
/// the per-account sync, push, and calendar runners; orchestrated
/// DB delete via `delete_account_orchestrate`; and external-store
/// cleanup. Folding all three closes the runner-quiescence
/// invariant Service-side: a future caller cannot delete while
/// runners hold writer references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeleteParams {
    pub account_id: String,
}

/// `account.delete` ack. Carries the cleanup report so the UI can
/// surface a "deleted N attachments / N body rows" affordance if it
/// wants. Today the UI only logs at info; the report's flat shape
/// (single struct, no nested errors) reflects that read pattern.
///
/// `cache_file_errors` is the per-path error list returned by the
/// attachment-cache cleanup. Each entry is `"<relative_path>:
/// <error>"`. Today the UI only logs the count; future surface
/// changes can render the list as a debug affordance without a
/// wire-shape change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeleteAck {
    pub bodies_deleted: u64,
    pub inline_images_deleted: u64,
    pub cache_files_deleted: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cache_file_errors: Vec<String>,
    pub search_cleaned: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_update_round_trip_full() {
        let original = AccountUpdateParams {
            id: "acc-1".into(),
            account_name: Some("Work".into()),
            display_name: Some("Atle".into()),
            account_color: Some("#abcdef".into()),
            caldav_url: Some("https://example.com/dav".into()),
            caldav_username: Some("atle".into()),
            caldav_password: Some("secret".into()),
            cache_attachments_enabled: Some(false),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_update_round_trip_partial_skips_none() {
        let original = AccountUpdateParams {
            id: "acc-1".into(),
            account_name: None,
            display_name: Some("Atle".into()),
            account_color: None,
            caldav_url: None,
            caldav_username: None,
            caldav_password: None,
            cache_attachments_enabled: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(!obj.contains_key("account_name"));
        assert!(obj.contains_key("display_name"));
        assert!(!obj.contains_key("account_color"));
        assert!(!obj.contains_key("caldav_url"));
        assert!(!obj.contains_key("caldav_username"));
        assert!(!obj.contains_key("caldav_password"));
        let recovered: AccountUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_reorder_round_trip() {
        let original = AccountReorderParams {
            orders: vec![
                AccountReorderEntry {
                    account_id: "a".into(),
                    sort_order: 0,
                },
                AccountReorderEntry {
                    account_id: "b".into(),
                    sort_order: 1,
                },
            ],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountReorderParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    fn sample_create_plaintext() -> AccountCreateParams {
        AccountCreateParams {
            email: "atle@example.com".into(),
            provider: "imap".into(),
            display_name: Some("Atle".into()),
            account_name: "Work".into(),
            account_color: "#abcdef".into(),
            auth_method: "password".into(),
            credentials: AccountCredentials::Plaintext {
                access_token: None,
                refresh_token: None,
                imap_password: Some("secret".into()),
                smtp_password: None,
            },
            token_expires_at: None,
            oauth_provider: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            oauth_extra_scopes: None,
            imap_host: Some("imap.example.com".into()),
            imap_port: Some(993),
            imap_security: Some("ssl".into()),
            imap_username: Some("atle".into()),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            smtp_security: Some("starttls".into()),
            smtp_username: None,
            jmap_url: None,
            accept_invalid_certs: false,
        }
    }

    #[test]
    fn account_create_plaintext_round_trip() {
        let original = sample_create_plaintext();
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountCreateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_create_encrypted_round_trip() {
        let original = AccountCreateParams {
            email: "atle@example.com".into(),
            provider: "gmail_api".into(),
            display_name: None,
            account_name: "Personal".into(),
            account_color: String::new(),
            auth_method: "oauth".into(),
            credentials: AccountCredentials::Encrypted {
                access_token: Some("enc:aaa:bbb".into()),
                refresh_token: Some("enc:ccc:ddd".into()),
                imap_password: None,
                smtp_password: None,
            },
            token_expires_at: Some(1_700_000_000),
            oauth_provider: Some("google".into()),
            oauth_client_id: Some("client-id-abc".into()),
            oauth_client_secret: Some("client-secret-xyz".into()),
            oauth_extra_scopes: Some("custom:scope another:one".into()),
            imap_host: None,
            imap_port: None,
            imap_security: None,
            imap_username: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            smtp_username: None,
            jmap_url: None,
            accept_invalid_certs: false,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountCreateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_create_credentials_kind_tag_disambiguates() {
        // Two structurally-identical Plaintext / Encrypted bodies must
        // serialize to distinct JSON shapes via the `kind` tag.
        let plain = AccountCredentials::Plaintext {
            access_token: Some("t".into()),
            refresh_token: None,
            imap_password: None,
            smtp_password: None,
        };
        let enc = AccountCredentials::Encrypted {
            access_token: Some("t".into()),
            refresh_token: None,
            imap_password: None,
            smtp_password: None,
        };
        let pj = serde_json::to_value(&plain).expect("p ser");
        let ej = serde_json::to_value(&enc).expect("e ser");
        assert_ne!(pj, ej);
        assert_eq!(pj["kind"], serde_json::json!("plaintext"));
        assert_eq!(ej["kind"], serde_json::json!("encrypted"));
    }

    #[test]
    fn account_create_ack_round_trips() {
        let original = AccountCreateAck {
            id: "acc-uuid-1".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountCreateAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_update_tokens_params_round_trip() {
        let original = AccountUpdateTokensParams {
            account_id: "acc-1".into(),
            access_token: Some(RedactedString::new("at")),
            refresh_token: Some(RedactedString::new("rt")),
            token_expires_at: Some(1234567890),
            imap_password: None,
            smtp_password: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountUpdateTokensParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_update_tokens_params_skips_none() {
        let original = AccountUpdateTokensParams {
            account_id: "acc-1".into(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            imap_password: Some(RedactedString::new("p")),
            smtp_password: None,
        };
        let formatted = format!("{original:?}");
        assert!(
            !formatted.contains("\"p\""),
            "Debug must not leak password: {formatted}",
        );
    }

    #[test]
    fn account_delete_params_round_trip() {
        let original = AccountDeleteParams {
            account_id: "acc-uuid-1".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountDeleteParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_delete_ack_round_trip_full() {
        let original = AccountDeleteAck {
            bodies_deleted: 5,
            inline_images_deleted: 3,
            cache_files_deleted: 2,
            cache_file_errors: vec!["a/b/c.png: permission denied".into()],
            search_cleaned: true,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountDeleteAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_delete_ack_skips_empty_errors() {
        let original = AccountDeleteAck::default();
        let json = serde_json::to_value(&original).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(
            !object.contains_key("cache_file_errors"),
            "empty cache_file_errors must be omitted: got {object:?}",
        );
    }
}
