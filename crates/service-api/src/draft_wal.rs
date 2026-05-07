//! Draft WAL wire shape (UI-side append + Service-side drain).
//!
//! The compose-draft auto-save and window-close paths append NDJSON
//! lines to `<data_dir>/drafts.wal` UI-side; the Service drains and
//! replays them on next boot during `BootPhase::DrainingDraftWal`.
//! Both crates serialize a `WalEntry { epoch_ms, params }` shape
//! today but neither imports the other's struct, so adding a field
//! to one side without the other surfaces as silent draft loss
//! (the Service's drain logs "skipping unparseable line" and moves
//! on).
//!
//! The constants here are the single source of truth for:
//! - `WAL_FILENAME`: the file basename both sides agree on.
//! - `DRAFT_WAL_GOLDEN_FIXTURE_JSON`: a canonical serialized
//!   `WalEntry` against a fixed `SaveLocalDraftParams`. Both crates'
//!   tests construct the same fixture inputs, serialize, and assert
//!   their bytes equal this constant. If either crate's `WalEntry`
//!   or `SaveLocalDraftParams` shape drifts, that crate's test
//!   breaks and the constant must be updated here in lockstep -
//!   the asymmetric "one side updates, the other doesn't notice"
//!   class is gone.

/// File basename of the active WAL inside the user data directory.
pub const WAL_FILENAME: &str = "drafts.wal";

/// Golden serialized `WalEntry` against the fixture inputs documented
/// in `DRAFT_WAL_GOLDEN_FIXTURE_*` below. Every field of
/// `SaveLocalDraftParams` is populated so a divergence in any field
/// surfaces. Keep this string byte-identical with the actual
/// `serde_json::to_string(&WalEntry)` output of both UI and Service.
pub const DRAFT_WAL_GOLDEN_FIXTURE_JSON: &str = concat!(
    r#"{"epoch_ms":1700000000000,"#,
    r#""params":{"id":"draft-fixture","#,
    r#""account_id":"acct-fixture","#,
    r#""to_addresses":"to@example.com","#,
    r#""cc_addresses":"cc@example.com","#,
    r#""bcc_addresses":"bcc@example.com","#,
    r#""subject":"fixture subject","#,
    r#""body_html":"<p>fixture body</p>","#,
    r#""reply_to_message_id":"msg-reply","#,
    r#""thread_id":"thread-fixture","#,
    r#""from_email":"me@example.com","#,
    r#""signature_id":"sig-1","#,
    r#""remote_draft_id":"remote-1","#,
    r#""attachments":"[]","#,
    r#""signature_separator_index":42}}"#,
);

/// Fixture epoch_ms used by the golden test on both sides.
pub const DRAFT_WAL_GOLDEN_FIXTURE_EPOCH_MS: u64 = 1_700_000_000_000;
