use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::{
    BooleanQuery, BoostQuery, Occur, Query, QueryParser, RangeQuery, TermQuery,
};
use tantivy::schema::{
    DateOptions, Field, NumericOptions, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::{DateTime as TantivyDateTime, Index, IndexReader, ReloadPolicy, Term};
use types::DateBound;

// ── Schema version sentinel ─────────────────────────────────────────────

/// Persisted at `<app_data>/search_index/.version`. Bumped when the
/// per-message Tantivy doc shape changes meaningfully:
///   - new field added to `build_schema()` -> bump (forces re-index of all
///     messages so the new field is populated).
///   - extractor output format changes -> bump (forces re-extraction of
///     all attachments so re-indexed docs reflect the new text).
///
/// The Service-side `boot::check_schema_version_and_dispatch` compares the
/// persisted value against this constant and dispatches a rebuild when
/// they differ. `open_or_create_search_index` is unaware of the version -
/// it is shared by the Service writer and the UI reader, so the
/// destructive rebuild path lives Service-side only.
///
/// Phase 7-1 landed this constant at value 1; phase 7-3 bumps to 2
/// because the per-message doc now carries `attachment_text` /
/// `attachment_filename` / `attachment_mime` / `attachment_id`
/// multi-value fields (one per cached + extracted attachment).
pub const INDEX_SCHEMA_VERSION: u32 = 2;

/// Belt-and-suspenders boundary padding inserted between attachment
/// values within the multi-value `attachment_text` field. Tantivy's
/// internal `POSITION_GAP=1` puts a 2-position gap between consecutive
/// `add_text` calls' tokens (last-of-prev to first-of-next), which
/// already blocks slop-0 phrase queries from straddling attachment
/// boundaries. Adding 32 boundary tokens between values defends
/// against slop>=2 phrase queries too. The token "rtskbnd" is chosen
/// to be unlikely-to-occur in real attachment text; a query for the
/// literal "rtskbnd" would match every multi-attachment message,
/// which is acceptable v1 noise (no human types this).
///
/// Cost: 32 * 8 bytes = 256 bytes per attachment-after-the-first per
/// message. With up to ~10 attachments per message and 100 KB text per
/// attachment, the boundary overhead is well under 1% of stored bytes.
pub(crate) const ATTACHMENT_BOUNDARY_TOKEN: &str = "rtskbnd";
pub(crate) const ATTACHMENT_BOUNDARY_REPEATS: usize = 32;

// ── Schema ──────────────────────────────────────────────────────────────

fn text_indexed_stored() -> TextOptions {
    TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("default")
                .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored()
}

fn text_indexed() -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    )
}

pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    // Stored + indexed text fields
    builder.add_text_field("subject", text_indexed_stored());
    builder.add_text_field("from_name", text_indexed_stored());
    builder.add_text_field("from_address", STRING | STORED);
    builder.add_text_field("to_addresses", text_indexed());
    builder.add_text_field("body_text", text_indexed());
    builder.add_text_field("snippet", text_indexed_stored());

    // Identifiers (stored, not tokenized)
    builder.add_text_field("message_id", STRING | STORED);
    builder.add_text_field("thread_id", STRING | STORED);
    builder.add_text_field("account_id", STRING | STORED);

    // Date field for range queries and sorting
    builder.add_date_field(
        "date",
        DateOptions::default().set_indexed().set_fast().set_stored(),
    );

    // Fast filter fields
    builder.add_u64_field(
        "is_read",
        NumericOptions::default()
            .set_indexed()
            .set_fast()
            .set_stored(),
    );
    builder.add_u64_field(
        "is_starred",
        NumericOptions::default()
            .set_indexed()
            .set_fast()
            .set_stored(),
    );
    builder.add_u64_field(
        "has_attachment",
        NumericOptions::default()
            .set_indexed()
            .set_fast()
            .set_stored(),
    );

    // Phase 7: per-attachment text + metadata. Multi-value fields
    // populated once per cached + extracted attachment per message.
    // Indexed-not-stored on attachment_text because per-attachment
    // snippet retrieval at search time goes through the SQLite
    // `attachment_extracted_text` table (see phase 7-8); Tantivy only
    // needs to know the text for matching, not for retrieval.
    builder.add_text_field("attachment_text", text_indexed());
    builder.add_text_field("attachment_filename", text_indexed_stored());
    builder.add_text_field("attachment_mime", STRING | STORED);
    builder.add_text_field("attachment_id", STRING | STORED);

    builder.build()
}

// ── Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchDocument {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachment: bool,
    /// Phase 7: per-attachment text + metadata fragments. Each
    /// fragment becomes one `add_text` call on each of the four
    /// `attachment_*` Tantivy fields (text, filename, mime, id) at
    /// build_search_doc time. Provider crates default this to empty;
    /// the writer task's apply-time enrichment (phase 7-3 §
    /// SearchDocument construction relocation) populates it by
    /// JOINing `attachment_extracted_text` against the message's
    /// attachments when the writer applies the command. Sync providers
    /// pass through thin docs; the writer fills in attachment data.
    #[serde(default)]
    pub attachments: Vec<AttachmentDocFragment>,
}

/// Per-attachment doc fragment. One per cached + extracted attachment
/// per message. The four fields populate the four `attachment_*`
/// Tantivy fields in lockstep so multi-value index ordinals align
/// across fields - reader code can iterate them in insertion order
/// to recover (filename, mime, id) tuples.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDocFragment {
    pub attachment_id:  String,
    pub filename:       String,
    pub mime:           String,
    pub extracted_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub snippet: Option<String>,
    pub date: i64,
    pub rank: f32,
    /// Phase 7: highest-scoring single field that matched the query.
    /// Defaults to `MatchKind::Body` until phase 7-8's per-field
    /// snippet generator + per-attachment attribution lands.
    #[serde(default)]
    pub match_kind: MatchKind,
    /// Phase 7: secondary matches above the score threshold. Empty
    /// until phase 7-8.
    #[serde(default)]
    pub also_matched: Vec<MatchKind>,
}

/// Phase 7: which field a search hit matched in. Phase 7-3 lands the
/// type with `Body` as the default; phase 7-8 populates per-result
/// match_kind via per-field snippet generation and per-attachment
/// attribution against `attachment_extracted_text`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MatchKind {
    #[default]
    Body,
    Subject,
    From,
    Attachment {
        attachment_id: String,
        filename:      String,
        mime:          String,
        snippet:       String,
    },
}

/// Phase 7-8: per-message inputs for `SearchReadState::enrich_match_kinds`.
/// The caller (typically `core::search_pipeline`) materialises these from
/// canonical DB state + the body store before invoking enrichment, so the
/// search crate stays free of DB dependencies.
#[derive(Debug, Clone, Default)]
pub struct AttributionInputs {
    pub subject:     String,
    pub from_name:   String,
    pub body_text:   String,
    pub attachments: Vec<AttachmentAttributionInput>,
}

/// Phase 7-8: per-attachment input for attribution scoring. `extracted_text`
/// is empty if the attachment has no `'indexed'` row in
/// `attachment_extracted_text` (skipped/failed extractions still appear in
/// the doc but contribute no full-text segment).
#[derive(Debug, Clone)]
pub struct AttachmentAttributionInput {
    pub attachment_id:  String,
    pub filename:       String,
    pub mime:           String,
    pub extracted_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    /// Account filter. `None` = search all accounts.
    /// `Some(ids)` = search only those accounts.
    pub account_ids: Option<Vec<String>>,
    pub free_text: Option<String>,
    /// From filters - multiple values produce OR semantics.
    pub from: Vec<String>,
    /// To filters - multiple values produce OR semantics.
    pub to: Vec<String>,
    pub subject: Option<String>,
    pub has_attachment: Option<bool>,
    pub is_unread: Option<bool>,
    pub is_starred: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_before_bound")]
    pub before: Option<DateBound>,
    #[serde(default, deserialize_with = "deserialize_after_bound")]
    pub after: Option<DateBound>,
    /// Restrict results to the given (account_id, thread_id) tuples. Used by
    /// the combined-path pipeline to pin Tantivy's response to the SQL
    /// candidate set, so that "SQL narrows the corpus, Tantivy ranks within
    /// it" is realised at the engine level rather than via post-hoc
    /// intersection in app code. `None` = no thread filter; `Some(empty)` =
    /// short-circuit to zero results (caller should not invoke Tantivy at
    /// all in that case).
    #[serde(default)]
    pub thread_filter: Option<Vec<(String, String)>>,
    pub limit: Option<usize>,
}

fn deserialize_before_bound<'de, D>(deserializer: D) -> Result<Option<DateBound>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(|value| value.map(DateBound::before))
}

fn deserialize_after_bound<'de, D>(deserializer: D) -> Result<Option<DateBound>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(|value| value.map(DateBound::after))
}

// ── Field helpers ───────────────────────────────────────────────────────

/// Pre-resolved Tantivy field handles. Kept `pub` so the Service-side
/// writer task (`crates/service/src/search_writer.rs`) can build
/// `TantivyDocument`s using the same schema as the read path.
#[derive(Clone, Debug)]
pub struct Fields {
    pub subject: Field,
    pub from_name: Field,
    pub from_address: Field,
    pub to_addresses: Field,
    pub body_text: Field,
    pub snippet: Field,
    pub message_id: Field,
    pub thread_id: Field,
    pub account_id: Field,
    pub date: Field,
    pub is_read: Field,
    pub is_starred: Field,
    pub has_attachment: Field,
    pub attachment_text: Field,
    pub attachment_filename: Field,
    pub attachment_mime: Field,
    pub attachment_id: Field,
}

impl Fields {
    pub fn from_schema(schema: &Schema) -> Self {
        Self {
            subject: schema.get_field("subject").expect("subject field"),
            from_name: schema.get_field("from_name").expect("from_name field"),
            from_address: schema
                .get_field("from_address")
                .expect("from_address field"),
            to_addresses: schema
                .get_field("to_addresses")
                .expect("to_addresses field"),
            body_text: schema.get_field("body_text").expect("body_text field"),
            snippet: schema.get_field("snippet").expect("snippet field"),
            message_id: schema.get_field("message_id").expect("message_id field"),
            thread_id: schema.get_field("thread_id").expect("thread_id field"),
            account_id: schema.get_field("account_id").expect("account_id field"),
            date: schema.get_field("date").expect("date field"),
            is_read: schema.get_field("is_read").expect("is_read field"),
            is_starred: schema.get_field("is_starred").expect("is_starred field"),
            has_attachment: schema
                .get_field("has_attachment")
                .expect("has_attachment field"),
            attachment_text: schema
                .get_field("attachment_text")
                .expect("attachment_text field"),
            attachment_filename: schema
                .get_field("attachment_filename")
                .expect("attachment_filename field"),
            attachment_mime: schema
                .get_field("attachment_mime")
                .expect("attachment_mime field"),
            attachment_id: schema
                .get_field("attachment_id")
                .expect("attachment_id field"),
        }
    }
}

// ── Index opener ────────────────────────────────────────────────────────

const DEFAULT_INDEX_DIR_NAME: &str = "search_index";
const ACTIVE_INDEX_FILE_NAME: &str = "search_index.active";

/// Resolve the active Tantivy index directory for `app_data_dir`.
///
/// The default slot is `{app_data_dir}/search_index`. PreserveExisting
/// rebuilds write a pointer file after the rebuilt staging slot is
/// caught up; new readers and the next boot open that active slot
/// without renaming a directory that an old reader may still have
/// mmaped.
pub fn active_search_index_dir(app_data_dir: &Path) -> std::path::PathBuf {
    if let Some(name) = read_active_index_dir_name(app_data_dir) {
        app_data_dir.join(name)
    } else {
        app_data_dir.join(DEFAULT_INDEX_DIR_NAME)
    }
}

/// Build a unique staging directory for a PreserveExisting rebuild.
pub fn staging_search_index_dir(app_data_dir: &Path, rebuild_id: &str) -> std::path::PathBuf {
    let safe_id: String = rebuild_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    app_data_dir.join(format!("search_index_next_{safe_id}"))
}

/// Atomically point future readers and boots at `index_dir`.
pub fn write_active_search_index_dir(
    app_data_dir: &Path,
    index_dir: &Path,
) -> Result<(), String> {
    let name = index_dir
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("active index path has no file name: {}", index_dir.display()))?;
    validate_active_index_dir_name(name)?;
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("create app data dir {}: {e}", app_data_dir.display()))?;
    let pointer = app_data_dir.join(ACTIVE_INDEX_FILE_NAME);
    let tmp = app_data_dir.join(format!("{ACTIVE_INDEX_FILE_NAME}.tmp"));
    std::fs::write(&tmp, name).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &pointer).map_err(|e| {
        format!(
            "rename active index pointer {} -> {}: {e}",
            tmp.display(),
            pointer.display(),
        )
    })?;
    Ok(())
}

fn read_active_index_dir_name(app_data_dir: &Path) -> Option<String> {
    let pointer = app_data_dir.join(ACTIVE_INDEX_FILE_NAME);
    let raw = match std::fs::read_to_string(&pointer) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!("read active search index pointer {}: {e}", pointer.display());
            return None;
        }
    };
    let name = raw.trim();
    if let Err(e) = validate_active_index_dir_name(name) {
        log::warn!(
            "ignoring invalid active search index pointer {}: {e}",
            pointer.display(),
        );
        return None;
    }
    Some(name.to_string())
}

fn validate_active_index_dir_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty index directory name".into());
    }
    if name == "." || name == ".." {
        return Err("index directory name must not be relative traversal".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("index directory name must not contain path separators".into());
    }
    Ok(())
}

/// Open or create the Tantivy index in the active search-index slot
/// for `app_data_dir`.
///
/// Phase 3 task 4 exposes this so the Service writer task and the UI's
/// `SearchReadState` can both consume the same opener (the writer task
/// runs in `BootPhase::OpeningSearchIndex` and creates the directory if
/// it does not exist; the read state opens after the boot handshake
/// completes).
pub fn open_or_create_search_index(app_data_dir: &Path) -> Result<(Index, Schema), String> {
    let index_dir = active_search_index_dir(app_data_dir);
    open_or_create_search_index_at(&index_dir)
}

/// Open or create the Tantivy index at an explicit directory.
pub fn open_or_create_search_index_at(index_dir: &Path) -> Result<(Index, Schema), String> {
    std::fs::create_dir_all(index_dir).map_err(|e| format!("create search index dir: {e}"))?;

    let schema = build_schema();

    log::info!("Opening search index at {}", index_dir.display());

    let index = if Index::exists(
        &tantivy::directory::MmapDirectory::open(index_dir)
            .map_err(|e| format!("open mmap dir: {e}"))?,
    )
    .map_err(|e| format!("check index exists: {e}"))?
    {
        log::info!("Opening existing search index");
        Index::open_in_dir(index_dir).map_err(|e| {
            log::error!("Failed to open search index: {e}");
            format!("open index: {e}")
        })?
    } else {
        log::info!("Creating new search index");
        Index::create_in_dir(index_dir, schema.clone()).map_err(|e| {
            log::error!("Failed to create search index: {e}");
            format!("create index: {e}")
        })?
    };

    Ok((index, schema))
}

/// Build a Tantivy document from a `SearchDocument` against the
/// pre-resolved field handles.
///
/// `pub` so the Service-side writer task (which owns the `IndexWriter`
/// post-task-4) can drive document construction without a circular
/// dependency on a `SearchReadState` reference.
pub fn build_search_doc(fields: &Fields, msg: &SearchDocument) -> tantivy::TantivyDocument {
    let mut doc = tantivy::TantivyDocument::default();

    doc.add_text(fields.message_id, &msg.message_id);
    doc.add_text(fields.account_id, &msg.account_id);
    doc.add_text(fields.thread_id, &msg.thread_id);
    doc.add_text(fields.subject, msg.subject.as_deref().unwrap_or_default());
    doc.add_text(
        fields.from_name,
        msg.from_name.as_deref().unwrap_or_default(),
    );
    doc.add_text(
        fields.from_address,
        msg.from_address.as_deref().unwrap_or_default(),
    );
    doc.add_text(
        fields.to_addresses,
        msg.to_addresses.as_deref().unwrap_or_default(),
    );
    doc.add_text(
        fields.body_text,
        msg.body_text.as_deref().unwrap_or_default(),
    );
    doc.add_text(fields.snippet, msg.snippet.as_deref().unwrap_or_default());
    doc.add_date(
        fields.date,
        TantivyDateTime::from_timestamp_secs(msg.date),
    );
    doc.add_u64(fields.is_read, u64::from(msg.is_read));
    doc.add_u64(fields.is_starred, u64::from(msg.is_starred));
    doc.add_u64(fields.has_attachment, u64::from(msg.has_attachment));

    // Phase 7: per-attachment text + metadata. One add_text per
    // attachment for each of the four attachment_* fields, ordered
    // identically across the four so multi-value ordinals align.
    // Boundary padding inserted into attachment_text-after-the-first
    // pushes positions further apart so cross-boundary phrase queries
    // (slop > 1) cannot match across attachments.
    if !msg.attachments.is_empty() {
        let boundary = build_boundary_string();
        for (i, att) in msg.attachments.iter().enumerate() {
            let text_value = if i == 0 {
                att.extracted_text.clone()
            } else {
                let mut s = String::with_capacity(boundary.len() + 1 + att.extracted_text.len());
                s.push_str(&boundary);
                s.push(' ');
                s.push_str(&att.extracted_text);
                s
            };
            doc.add_text(fields.attachment_text, &text_value);
            doc.add_text(fields.attachment_filename, &att.filename);
            doc.add_text(fields.attachment_mime, &att.mime);
            doc.add_text(fields.attachment_id, &att.attachment_id);
        }
    }

    doc
}

/// Build the boundary-token string injected between attachment text
/// values. Computed once per call site rather than via a OnceLock to
/// keep the search crate dependency-free of `std::sync::OnceLock`
/// (which is fine, but the cost of building a 256-byte string is
/// negligible).
fn build_boundary_string() -> String {
    let mut s = String::with_capacity(
        ATTACHMENT_BOUNDARY_REPEATS * (ATTACHMENT_BOUNDARY_TOKEN.len() + 1),
    );
    for i in 0..ATTACHMENT_BOUNDARY_REPEATS {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(ATTACHMENT_BOUNDARY_TOKEN);
    }
    s
}

// ── SearchReadState ─────────────────────────────────────────────────────────

/// Read-only handle to the Tantivy search index.
///
/// Phase 3 task 4 strips writer ownership from this type. Writes flow
/// through `service-state::SearchWriteHandle` (mpsc) to a Service-only
/// writer task that owns the single `IndexWriter`. Tantivy's per-
/// directory writer lock enforces the single-writer invariant; the
/// `SearchReadState` only opens the reader.
#[derive(Clone)]
pub struct SearchReadState {
    reader: IndexReader,
    fields: Fields,
}

impl std::fmt::Debug for SearchReadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchReadState").finish_non_exhaustive()
    }
}

// IndexReader is Send + Sync; no interior mutability that would block auto-derives.

impl SearchReadState {
    /// Open the search index reader for `{app_data_dir}/search_index/`.
    ///
    /// The boot ordering contract (Phase 3 task 12): the Service spawns
    /// the writer task in `BootPhase::OpeningSearchIndex` *before* the
    /// boot handshake completes; the UI constructs `SearchReadState` only
    /// after `boot.ready`, so the directory is guaranteed to exist and
    /// hold at least the initial empty segment.
    ///
    /// This entry point still calls `open_or_create_search_index` for the
    /// Service-internal callers (action service constructs a read state
    /// for its `ProviderCtx` builder) and for tests.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        let (index, schema) = open_or_create_search_index(app_data_dir)?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("create reader: {e}"))?;

        let fields = Fields::from_schema(&schema);

        Ok(Self { reader, fields })
    }

    /// Drop the cached `Searcher` so subsequent searches see the
    /// latest committed segments. Cheap; tantivy reloads cooperatively
    /// on the next searcher acquire.
    pub fn reload(&self) -> Result<(), String> {
        self.reader
            .reload()
            .map_err(|e| format!("reader reload: {e}"))
    }

    /// Field handle accessor (used by tests + by future read paths
    /// that need to inspect the schema).
    pub fn fields(&self) -> &Fields {
        &self.fields
    }

    /// Exposed for the Phase 3 startup invariant pass: returns whether
    /// a document with the given `message_id` exists in the index.
    pub fn message_indexed(&self, message_id: &str) -> Result<bool, String> {
        let searcher = self.reader.searcher();
        let term = Term::from_field_text(self.fields.message_id, message_id);
        let q = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
        let count = searcher
            .search(&q, &tantivy::collector::Count)
            .map_err(|e| format!("search: {e}"))?;
        Ok(count > 0)
    }

    /// Phase 8-2: enumerate `message_id`s in the search index that
    /// belong to `account_id` but are absent from `live_message_ids`.
    /// Used by the startup invariant pass to drop Tantivy docs whose
    /// underlying `messages` row was deleted in a non-graceful exit
    /// window.
    ///
    /// Bounded by the dirty-account scope - the caller restricts
    /// invocations to accounts whose `sync_markers/<id>.json` survived
    /// (i.e., the previous sync did not finalize). On a typical account
    /// this returns quickly because the live set covers nearly every
    /// indexed message.
    pub fn find_orphan_message_ids_for_account(
        &self,
        account_id: &str,
        live_message_ids: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>, String> {
        let searcher = self.reader.searcher();
        let term = Term::from_field_text(self.fields.account_id, account_id);
        let q = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
        // First find how many docs match, then collect that many. Two
        // queries against the same searcher are cheap (the segments are
        // already mmaped) and let us size TopDocs precisely instead of
        // guessing at a cap.
        let count = searcher
            .search(&q, &tantivy::collector::Count)
            .map_err(|e| format!("orphan count: {e}"))?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let top = searcher
            .search(&q, &TopDocs::with_limit(count).order_by_score())
            .map_err(|e| format!("orphan iter: {e}"))?;
        let mut orphans = Vec::new();
        for (_score, addr) in top {
            let doc: tantivy::TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| format!("orphan doc fetch: {e}"))?;
            if let Some(mid) = Self::get_text(&doc, self.fields.message_id)
                && !live_message_ids.contains(&mid)
            {
                orphans.push(mid);
            }
        }
        Ok(orphans)
    }

    /// Search with structured filters.
    #[allow(clippy::too_many_lines)]
    pub fn search_with_filters(&self, params: &SearchParams) -> Result<Vec<SearchResult>, String> {
        log::debug!(
            "Searching with filters: free_text={:?}, from_count={}, to_count={}, limit={:?}",
            params.free_text,
            params.from.len(),
            params.to.len(),
            params.limit,
        );
        let searcher = self.reader.searcher();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Filter by account IDs (None = all accounts)
        if let Some(filter) = self.build_account_filter(params.account_ids.as_deref()) {
            clauses.push((Occur::Must, filter));
        }

        // Free text → QueryParser on subject+from_name+to_addresses+
        // body_text+snippet plus attachment_text and attachment_filename so
        // attachment-only matches can enter the result set. The per-attachment
        // attribution pass downstream of this query (`enrich_match_kinds`)
        // annotates hits as `MatchKind::Attachment` when the attachment fields
        // scored higher than the body fields. `from_address` is intentionally
        // omitted - the field is indexed as `STRING` (untokenized) for exact
        // matching by the `from:` operator, so it cannot participate in
        // tokenized free-text search.
        if let Some(ref text) = params.free_text
            && !text.is_empty()
        {
            let qp = QueryParser::for_index(
                searcher.index(),
                vec![
                    self.fields.subject,
                    self.fields.from_name,
                    self.fields.to_addresses,
                    self.fields.body_text,
                    self.fields.snippet,
                    self.fields.attachment_text,
                    self.fields.attachment_filename,
                ],
            );
            let q = qp
                .parse_query(text)
                .map_err(|e| format!("parse free text: {e}"))?;
            clauses.push((Occur::Must, q));
        }

        // from → OR across all from values (address term OR name phrase)
        if !params.from.is_empty() {
            let mut from_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            let from_name_qp =
                QueryParser::for_index(searcher.index(), vec![self.fields.from_name]);
            for from_val in &params.from {
                if from_val.is_empty() {
                    continue;
                }
                let from_addr: Box<dyn Query> = Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.from_address, from_val),
                    tantivy::schema::IndexRecordOption::Basic,
                ));
                let from_name = from_name_qp
                    .parse_query(from_val)
                    .map_err(|e| format!("parse from: {e}"))?;
                from_clauses.push((Occur::Should, from_addr));
                from_clauses.push((Occur::Should, from_name));
            }
            if !from_clauses.is_empty() {
                clauses.push((Occur::Must, Box::new(BooleanQuery::new(from_clauses))));
            }
        }

        // to → OR across all to values
        if !params.to.is_empty() {
            let mut to_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            let to_qp = QueryParser::for_index(searcher.index(), vec![self.fields.to_addresses]);
            for to_val in &params.to {
                if to_val.is_empty() {
                    continue;
                }
                let q = to_qp
                    .parse_query(to_val)
                    .map_err(|e| format!("parse to: {e}"))?;
                to_clauses.push((Occur::Should, q));
            }
            if !to_clauses.is_empty() {
                clauses.push((Occur::Must, Box::new(BooleanQuery::new(to_clauses))));
            }
        }

        // subject → phrase on subject
        if let Some(ref subject) = params.subject
            && !subject.is_empty()
        {
            let subj_qp = QueryParser::for_index(searcher.index(), vec![self.fields.subject]);
            let q = subj_qp
                .parse_query(subject)
                .map_err(|e| format!("parse subject: {e}"))?;
            clauses.push((Occur::Must, q));
        }

        // has_attachment flag
        if let Some(has_att) = params.has_attachment {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(self.fields.has_attachment, u64::from(has_att)),
                    tantivy::schema::IndexRecordOption::Basic,
                )),
            ));
        }

        // is_unread flag (inverted: is_unread = !is_read)
        if let Some(is_unread) = params.is_unread {
            let is_read_val = u64::from(!is_unread);
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(self.fields.is_read, is_read_val),
                    tantivy::schema::IndexRecordOption::Basic,
                )),
            ));
        }

        // is_starred flag
        if let Some(is_starred) = params.is_starred {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(self.fields.is_starred, u64::from(is_starred)),
                    tantivy::schema::IndexRecordOption::Basic,
                )),
            ));
        }

        // Date range bounds are emitted by DateBound so SQL and Tantivy share
        // the same inclusivity semantics.
        if params.after.is_some() || params.before.is_some() {
            let lower = params
                .after
                .map(|bound| {
                    bound.to_range_bound(|ts| Term::from_field_date(
                        self.fields.date,
                        TantivyDateTime::from_timestamp_secs(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            let upper = params
                .before
                .map(|bound| {
                    bound.to_range_bound(|ts| Term::from_field_date(
                        self.fields.date,
                        TantivyDateTime::from_timestamp_secs(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
        }

        // Thread filter: pin the result set to a given list of
        // (account_id, thread_id) tuples. Used by the combined-path pipeline
        // so that Tantivy ranks within the SQL candidate set rather than
        // returning a broad global top-N that's then intersected in app
        // code.
        //
        // The whole filter clause is wrapped in BoostQuery(_, 0.0) so the
        // (account_id, thread_id) terms don't contribute to BM25 - we want
        // pure free-text scoring within the candidate set, not ranking
        // perturbed by per-account or per-thread term-frequency variance.
        if let Some(threads) = params.thread_filter.as_deref() {
            if threads.is_empty() {
                // Caller signalled "no candidates" - skip Tantivy entirely.
                return Ok(Vec::new());
            }
            let mut thread_clauses: Vec<(Occur, Box<dyn Query>)> =
                Vec::with_capacity(threads.len());
            for (account_id, thread_id) in threads {
                let pair: Vec<(Occur, Box<dyn Query>)> = vec![
                    (
                        Occur::Must,
                        Box::new(TermQuery::new(
                            Term::from_field_text(self.fields.account_id, account_id),
                            tantivy::schema::IndexRecordOption::Basic,
                        )),
                    ),
                    (
                        Occur::Must,
                        Box::new(TermQuery::new(
                            Term::from_field_text(self.fields.thread_id, thread_id),
                            tantivy::schema::IndexRecordOption::Basic,
                        )),
                    ),
                ];
                thread_clauses.push((Occur::Should, Box::new(BooleanQuery::new(pair))));
            }
            let filter: Box<dyn Query> = Box::new(BooleanQuery::new(thread_clauses));
            clauses.push((Occur::Must, Box::new(BoostQuery::new(filter, 0.0))));
        }

        let limit = params.limit.unwrap_or(50);

        if clauses.is_empty() {
            return Ok(Vec::new());
        }

        let combined = BooleanQuery::new(clauses);
        let top_docs = searcher
            .search(&combined, &TopDocs::with_limit(limit).order_by_score())
            .map_err(|e| format!("search: {e}"))?;

        let results = self.collect_results(&searcher, &top_docs)?;
        log::debug!("Search returned {} results", results.len());
        Ok(results)
    }

    /// Build an account filter query from optional account IDs.
    ///
    /// - `None` → no filter (search all accounts)
    /// - `Some(&[])` → no filter (empty slice treated as all)
    /// - `Some(&[id])` → single `TermQuery`
    /// - `Some(&[id1, id2, ...])` → `BooleanQuery` with `Should` clauses
    fn build_account_filter(&self, account_ids: Option<&[String]>) -> Option<Box<dyn Query>> {
        let ids = account_ids?;
        match ids.len() {
            0 => None,
            1 => Some(Box::new(TermQuery::new(
                Term::from_field_text(self.fields.account_id, &ids[0]),
                tantivy::schema::IndexRecordOption::Basic,
            ))),
            _ => {
                let sub: Vec<(Occur, Box<dyn Query>)> = ids
                    .iter()
                    .map(|id| -> (Occur, Box<dyn Query>) {
                        (
                            Occur::Should,
                            Box::new(TermQuery::new(
                                Term::from_field_text(self.fields.account_id, id),
                                tantivy::schema::IndexRecordOption::Basic,
                            )),
                        )
                    })
                    .collect();
                Some(Box::new(BooleanQuery::new(sub)))
            }
        }
    }

    /// Extract a text value from a tantivy document field.
    fn get_text(doc: &tantivy::TantivyDocument, field: Field) -> Option<String> {
        use tantivy::schema::document::Value;
        doc.get_first(field)
            .and_then(|v| v.as_value().as_str().map(String::from))
    }

    /// Extract a date value (as unix seconds) from a tantivy document field.
    fn get_date_secs(doc: &tantivy::TantivyDocument, field: Field) -> i64 {
        use tantivy::schema::document::Value;
        doc.get_first(field)
            .and_then(|v| {
                v.as_value()
                    .as_datetime()
                    .map(tantivy::DateTime::into_timestamp_secs)
            })
            .unwrap_or(0)
    }

    /// Extract search results from scored doc addresses.
    fn collect_results(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: &[(f32, tantivy::DocAddress)],
    ) -> Result<Vec<SearchResult>, String> {
        let mut results = Vec::with_capacity(top_docs.len());

        for &(score, doc_address) in top_docs {
            let doc: tantivy::TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| format!("retrieve doc: {e}"))?;

            results.push(SearchResult {
                message_id: Self::get_text(&doc, self.fields.message_id).unwrap_or_default(),
                account_id: Self::get_text(&doc, self.fields.account_id).unwrap_or_default(),
                thread_id: Self::get_text(&doc, self.fields.thread_id).unwrap_or_default(),
                subject: Self::get_text(&doc, self.fields.subject),
                from_name: Self::get_text(&doc, self.fields.from_name),
                from_address: Self::get_text(&doc, self.fields.from_address),
                snippet: Self::get_text(&doc, self.fields.snippet),
                date: Self::get_date_secs(&doc, self.fields.date),
                rank: score,
                // Default; replaced by `enrich_match_kinds` (7-8) when
                // the caller has DB access and supplies attribution
                // inputs. Operator-only or empty queries leave it.
                match_kind: MatchKind::Body,
                also_matched: Vec::new(),
            });
        }

        Ok(results)
    }

    /// Phase 7-8: rewrite each result's `match_kind` and `also_matched`
    /// to reflect which field actually carried the query match.
    ///
    /// `free_text` is the text portion of the user's query (everything
    /// outside structured operators). When empty or whitespace, this is
    /// a no-op; results retain their default `MatchKind::Body`.
    ///
    /// `inputs` is keyed by `message_id` and supplies the texts needed
    /// for per-field snippet generation: `body_text` is fetched from
    /// the body store (Tantivy doesn't store it), and the attachment
    /// list is the canonical DB state at search time.
    ///
    /// Algorithm:
    /// 1. Build a `SnippetGenerator` per text field (subject, from,
    ///    body, attachment) with a single-field `QueryParser`.
    /// 2. For each result, score every candidate (the three message
    ///    fields plus each attachment). Score = number of highlighted
    ///    ranges in the best fragment. Zero-score candidates are
    ///    dropped.
    /// 3. Tiebreak across attachments: total query-term occurrences in
    ///    the segment, then alphabetical filename for determinism.
    /// 4. Highest-scoring candidate becomes `match_kind`; remaining
    ///    candidates whose score is at least 50 % of the top score
    ///    become `also_matched`, score-descending.
    ///
    /// Errors during per-field query parsing log a warning and skip
    /// that field for the result; the function never propagates a
    /// parse failure. A doc-level error (missing inputs, etc.) leaves
    /// the result's default `MatchKind::Body` in place.
    pub fn enrich_match_kinds(
        &self,
        free_text: &str,
        results: &mut [SearchResult],
        inputs: &HashMap<String, AttributionInputs>,
    ) -> Result<(), String> {
        let trimmed = free_text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let searcher = self.reader.searcher();

        let body_gen = build_field_snippet_gen(&searcher, trimmed, self.fields.body_text);
        let subject_gen = build_field_snippet_gen(&searcher, trimmed, self.fields.subject);
        let from_gen = build_field_snippet_gen(&searcher, trimmed, self.fields.from_name);
        let att_gen = build_field_snippet_gen(&searcher, trimmed, self.fields.attachment_text);

        // Lower-cased, deduped query tokens for the per-attachment
        // term-frequency tiebreak.
        let mut query_tokens: Vec<String> = trimmed
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        query_tokens.sort();
        query_tokens.dedup();

        for result in results.iter_mut() {
            let Some(input) = inputs.get(&result.message_id) else {
                continue;
            };

            let body_score = score_field(&body_gen, &input.body_text);
            let subject_score = score_field(&subject_gen, &input.subject);
            let from_score = score_field(&from_gen, &input.from_name);

            // Per-attachment (highlights, term_freq, snippet).
            let mut att_scored: Vec<(usize, usize, String, &AttachmentAttributionInput)> =
                Vec::with_capacity(input.attachments.len());
            for att in &input.attachments {
                if att.extracted_text.is_empty() {
                    continue;
                }
                let Some(att_snippet_gen) = att_gen.as_ref() else {
                    continue;
                };
                let snippet = att_snippet_gen.snippet(&att.extracted_text);
                let highlights = snippet.highlighted().len();
                if highlights == 0 {
                    continue;
                }
                let lower = att.extracted_text.to_lowercase();
                let term_freq: usize = query_tokens
                    .iter()
                    .map(|t| count_word_occurrences(&lower, t))
                    .sum();
                att_scored.push((highlights, term_freq, snippet.fragment().to_string(), att));
            }
            // Sort: highlights desc, term_freq desc, filename asc.
            att_scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| a.3.filename.cmp(&b.3.filename))
            });

            let mut candidates: Vec<(MatchKind, usize)> = Vec::new();
            if body_score > 0 {
                candidates.push((MatchKind::Body, body_score));
            }
            if subject_score > 0 {
                candidates.push((MatchKind::Subject, subject_score));
            }
            if from_score > 0 {
                candidates.push((MatchKind::From, from_score));
            }
            for (highlights, _, fragment, att) in att_scored {
                candidates.push((
                    MatchKind::Attachment {
                        attachment_id: att.attachment_id.clone(),
                        filename:      att.filename.clone(),
                        mime:          att.mime.clone(),
                        snippet:       fragment,
                    },
                    highlights,
                ));
            }
            // Stable sort by score desc; equal scores preserve insertion
            // order (body > subject > from > attachments), matching the
            // plan's deterministic ordering.
            candidates.sort_by_key(|c| std::cmp::Reverse(c.1));

            let Some((primary, top_score)) = candidates.first().cloned() else {
                continue;
            };
            // 50% threshold: secondary candidates must score at least
            // half the top score, with floor=1. `top_score / 2` rounds
            // down (M3 fix: was `div_ceil(2)` which rounded *up*, so
            // odd top scores tightened the threshold to 60-66% and
            // dropped candidates the plan called for). `.max(1)`
            // keeps a top of 1 admitting every nonzero secondary.
            let threshold = (top_score / 2).max(1);
            let also: Vec<MatchKind> = candidates
                .into_iter()
                .skip(1)
                .filter(|(_, s)| *s >= threshold)
                .map(|(k, _)| k)
                .collect();
            result.match_kind = primary;
            result.also_matched = also;
        }
        Ok(())
    }
}

/// Build a per-field SnippetGenerator. Returns `None` on parse failure
/// (the caller logs and skips that field for affected results).
fn build_field_snippet_gen(
    searcher: &tantivy::Searcher,
    free_text: &str,
    field: Field,
) -> Option<SnippetGenerator> {
    let qp = QueryParser::for_index(searcher.index(), vec![field]);
    let query = match qp.parse_query(free_text) {
        Ok(q) => q,
        Err(e) => {
            log::debug!("enrich_match_kinds: parse failure on field: {e}");
            return None;
        }
    };
    match SnippetGenerator::create(searcher, &*query, field) {
        Ok(snippet_gen) => Some(snippet_gen),
        Err(e) => {
            log::debug!("enrich_match_kinds: snippet gen create failed: {e}");
            None
        }
    }
}

/// Run `snippet_gen.snippet(text)` and return `highlighted.len()` as
/// the score. Empty text or absent generator yields 0.
fn score_field(snippet_gen: &Option<SnippetGenerator>, text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let Some(snippet_gen) = snippet_gen else { return 0 };
    snippet_gen.snippet(text).highlighted().len()
}

/// Count whole-word occurrences of `needle` (already lowercase) in
/// `haystack` (already lowercase). Word boundaries are
/// non-alphanumeric characters or the start/end of the string. Used
/// only as a per-attachment tiebreak; precision is not critical.
///
/// L2 fix: pre-fix the boundary check ran on bytes via
/// `is_ascii_alphanumeric`. Multi-byte UTF-8 (`é = 0xC3 0xA9`) starts
/// with a non-ASCII byte, which then registered as a word boundary.
/// Latin-Extended haystacks ("café") would inflate term_freq for any
/// needle preceded by an accented character. Char-based boundary
/// checks via `char::is_alphanumeric` handle the full Unicode plane.
fn count_word_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() || haystack.is_empty() {
        return 0;
    }
    let nlen = needle.len();
    let mut count = 0usize;
    let mut idx = 0usize;
    while let Some(found) = haystack[idx..].find(needle) {
        let abs = idx + found;
        let before_ok = abs == 0
            || !haystack[..abs]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        let after = abs + nlen;
        let after_ok = after >= haystack.len()
            || !haystack[after..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if before_ok && after_ok {
            count += 1;
        }
        idx = abs + nlen.max(1);
    }
    count
}

/// Group message-level search results by `thread_id`, keeping the
/// highest-scoring result per thread. Returns one `SearchResult` per
/// unique `thread_id`, sorted by rank descending.
pub fn group_by_thread(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut best: HashMap<String, SearchResult> = HashMap::new();

    for result in results {
        best.entry(result.thread_id.clone())
            .and_modify(|existing| {
                if result.rank > existing.rank {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    let mut grouped: Vec<SearchResult> = best.into_values().collect();
    grouped.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(thread_id: &str, rank: f32) -> SearchResult {
        SearchResult {
            message_id: format!("msg-{thread_id}-{rank}"),
            account_id: "acct1".into(),
            thread_id: thread_id.into(),
            subject: None,
            from_name: None,
            from_address: None,
            snippet: None,
            date: 0,
            rank,
            match_kind: MatchKind::Body,
            also_matched: Vec::new(),
        }
    }

    // ── group_by_thread tests ────────────────────────────────────────

    #[test]
    fn group_by_thread_empty() {
        let grouped = group_by_thread(vec![]);
        assert!(grouped.is_empty());
    }

    #[test]
    fn group_by_thread_keeps_highest_score() {
        let results = vec![
            make_result("t1", 1.0),
            make_result("t1", 5.0),
            make_result("t1", 3.0),
            make_result("t2", 2.0),
            make_result("t2", 4.0),
        ];

        let grouped = group_by_thread(results);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].thread_id, "t1");
        assert!((grouped[0].rank - 5.0).abs() < f32::EPSILON);
        assert_eq!(grouped[1].thread_id, "t2");
        assert!((grouped[1].rank - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn group_by_thread_single_message_per_thread() {
        let results = vec![
            make_result("t1", 3.0),
            make_result("t2", 1.0),
            make_result("t3", 2.0),
        ];

        let grouped = group_by_thread(results);
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].thread_id, "t1");
        assert_eq!(grouped[1].thread_id, "t3");
        assert_eq!(grouped[2].thread_id, "t2");
    }

    fn build_date_boundary_test_index() -> SearchReadState {
        let schema = build_schema();
        let fields = Fields::from_schema(&schema);
        let index = Index::create_in_ram(schema);
        let mut writer: tantivy::IndexWriter = index.writer(15_000_000).expect("writer");
        for (message_id, date) in [("before", 999), ("exact", 1000), ("after", 1001)] {
            let doc = SearchDocument {
                message_id: message_id.into(),
                account_id: "acct1".into(),
                thread_id: format!("thread-{message_id}"),
                subject: Some("boundary".into()),
                from_name: Some("alice".into()),
                from_address: Some("alice@example.com".into()),
                to_addresses: None,
                body_text: Some("boundary marker".into()),
                snippet: Some("boundary marker".into()),
                date,
                is_read: false,
                is_starred: false,
                has_attachment: false,
                attachments: Vec::new(),
            };
            writer.add_document(build_search_doc(&fields, &doc)).expect("add");
        }
        writer.commit().expect("commit");
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .expect("reader");
        SearchReadState { reader, fields }
    }

    fn date_boundary_params(after: Option<DateBound>, before: Option<DateBound>) -> SearchParams {
        SearchParams {
            account_ids: None,
            free_text: Some("boundary".into()),
            from: Vec::new(),
            to: Vec::new(),
            subject: None,
            has_attachment: None,
            is_unread: None,
            is_starred: None,
            before,
            after,
            thread_filter: None,
            limit: Some(10),
        }
    }

    #[test]
    fn date_bounds_exclude_exact_threshold() {
        let read = build_date_boundary_test_index();

        let after_results = read
            .search_with_filters(&date_boundary_params(Some(DateBound::after(1000)), None))
            .expect("after search");
        let after_ids: Vec<&str> = after_results
            .iter()
            .map(|result| result.message_id.as_str())
            .collect();
        assert_eq!(after_ids, vec!["after"]);

        let before_results = read
            .search_with_filters(&date_boundary_params(None, Some(DateBound::before(1000))))
            .expect("before search");
        let before_ids: Vec<&str> = before_results
            .iter()
            .map(|result| result.message_id.as_str())
            .collect();
        assert_eq!(before_ids, vec!["before"]);
    }

    // ── Multi-account search tests ───────────────────────────────────
    //
    // The pre-Phase-3 read+write tests lived on `SearchState`; Phase 3
    // task 4 strips writer ownership from `SearchReadState` so the
    // search-with-data tests now live alongside the writer task in
    // `crates/service/src/search_writer.rs`.

    // ── Phase 7-3 verification ───────────────────────────────────────

    // ── Phase 7-8 attribution tests ──────────────────────────────────

    /// Build a one-message index whose body / subject / from / two
    /// attachments carry distinct, controllable text. Returns a
    /// `SearchReadState` ready for `enrich_match_kinds`.
    fn build_attribution_test_index(
        body: &str,
        subject: &str,
        from_name: &str,
        att1_text: &str,
        att2_text: &str,
    ) -> SearchReadState {
        use tantivy::Index;
        let schema = build_schema();
        let fields = Fields::from_schema(&schema);
        let index = Index::create_in_ram(schema.clone());
        let mut writer: tantivy::IndexWriter = index
            .writer(15_000_000)
            .expect("writer");
        let doc = SearchDocument {
            message_id: "msg1".into(),
            account_id: "acct1".into(),
            thread_id: "t1".into(),
            subject: Some(subject.into()),
            from_name: Some(from_name.into()),
            from_address: Some("alice@example.com".into()),
            to_addresses: Some("bob@example.com".into()),
            body_text: Some(body.into()),
            snippet: Some(body.chars().take(80).collect::<String>()),
            date: 1,
            is_read: false,
            is_starred: false,
            has_attachment: true,
            attachments: vec![
                AttachmentDocFragment {
                    attachment_id: "att1".into(),
                    filename: "first.pdf".into(),
                    mime: "application/pdf".into(),
                    extracted_text: att1_text.into(),
                },
                AttachmentDocFragment {
                    attachment_id: "att2".into(),
                    filename: "second.pdf".into(),
                    mime: "application/pdf".into(),
                    extracted_text: att2_text.into(),
                },
            ],
        };
        let tantivy_doc = build_search_doc(&fields, &doc);
        writer.add_document(tantivy_doc).expect("add");
        writer.commit().expect("commit");
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .expect("reader");
        SearchReadState { reader, fields }
    }

    fn make_result_for_attribution() -> SearchResult {
        SearchResult {
            message_id: "msg1".into(),
            account_id: "acct1".into(),
            thread_id: "t1".into(),
            subject: Some("dummy".into()),
            from_name: None,
            from_address: None,
            snippet: None,
            date: 1,
            rank: 1.0,
            match_kind: MatchKind::Body,
            also_matched: Vec::new(),
        }
    }

    fn make_inputs(
        body: &str,
        subject: &str,
        from_name: &str,
        att1_text: &str,
        att2_text: &str,
    ) -> HashMap<String, AttributionInputs> {
        let mut inputs = HashMap::new();
        inputs.insert(
            "msg1".to_string(),
            AttributionInputs {
                subject: subject.into(),
                from_name: from_name.into(),
                body_text: body.into(),
                attachments: vec![
                    AttachmentAttributionInput {
                        attachment_id:  "att1".into(),
                        filename:       "first.pdf".into(),
                        mime:           "application/pdf".into(),
                        extracted_text: att1_text.into(),
                    },
                    AttachmentAttributionInput {
                        attachment_id:  "att2".into(),
                        filename:       "second.pdf".into(),
                        mime:           "application/pdf".into(),
                        extracted_text: att2_text.into(),
                    },
                ],
            },
        );
        inputs
    }

    #[test]
    fn phase_7_8_empty_query_is_noop() {
        let read = build_attribution_test_index("hello world", "subj", "alice", "", "");
        let mut results = vec![make_result_for_attribution()];
        let inputs = make_inputs("hello world", "subj", "alice", "", "");
        read.enrich_match_kinds("   ", &mut results, &inputs).expect("ok");
        assert!(matches!(results[0].match_kind, MatchKind::Body));
        assert!(results[0].also_matched.is_empty());
    }

    #[test]
    fn phase_7_8_attachment_only_match_picks_filename_and_snippet() {
        let read = build_attribution_test_index(
            "ordinary email body unrelated",
            "general inquiry",
            "alice",
            "the quarterly contract specifies penalties for delay",
            "shipping manifest only - no contract here",
        );
        let mut results = vec![make_result_for_attribution()];
        let inputs = make_inputs(
            "ordinary email body unrelated",
            "general inquiry",
            "alice",
            "the quarterly contract specifies penalties for delay",
            "shipping manifest only - no contract here",
        );
        read.enrich_match_kinds("contract", &mut results, &inputs).expect("ok");
        match &results[0].match_kind {
            MatchKind::Attachment { filename, snippet, .. } => {
                // Both attachments contain "contract"; picker is by score
                // first, term-frequency next, then alphabetical filename.
                assert!(
                    filename == "first.pdf" || filename == "second.pdf",
                    "expected one of the two attachments, got {filename}"
                );
                assert!(
                    snippet.to_lowercase().contains("contract"),
                    "snippet should include matched term: {snippet}"
                );
            }
            other => panic!("expected Attachment match_kind, got {other:?}"),
        }
        // The other attachment should appear in also_matched (both contain
        // "contract" once).
        assert!(
            !results[0].also_matched.is_empty(),
            "second attachment should appear in also_matched"
        );
    }

    #[test]
    fn phase_7_8_body_match_is_primary() {
        let read = build_attribution_test_index(
            "the contract was signed contract contract again",
            "subject without it",
            "alice",
            "no relevant words here",
            "another irrelevant attachment",
        );
        let mut results = vec![make_result_for_attribution()];
        let inputs = make_inputs(
            "the contract was signed contract contract again",
            "subject without it",
            "alice",
            "no relevant words here",
            "another irrelevant attachment",
        );
        read.enrich_match_kinds("contract", &mut results, &inputs).expect("ok");
        assert!(matches!(results[0].match_kind, MatchKind::Body));
        assert!(
            results[0].also_matched.is_empty(),
            "no other field has the term: {:?}",
            results[0].also_matched
        );
    }

    #[test]
    fn phase_7_8_attachment_tiebreak_alphabetical_when_term_freq_equal() {
        let read = build_attribution_test_index(
            "no body text here",
            "no subject term",
            "alice",
            "contract once",
            "contract once",
        );
        let mut results = vec![make_result_for_attribution()];
        let inputs = make_inputs(
            "no body text here",
            "no subject term",
            "alice",
            "contract once",
            "contract once",
        );
        read.enrich_match_kinds("contract", &mut results, &inputs).expect("ok");
        // Tiebreak: alphabetical by filename. "first.pdf" < "second.pdf".
        match &results[0].match_kind {
            MatchKind::Attachment { filename, .. } => assert_eq!(filename, "first.pdf"),
            other => panic!("expected Attachment match_kind, got {other:?}"),
        }
        // The other attachment is included via also_matched.
        assert_eq!(results[0].also_matched.len(), 1);
        match &results[0].also_matched[0] {
            MatchKind::Attachment { filename, .. } => assert_eq!(filename, "second.pdf"),
            other => panic!("expected Attachment in also_matched, got {other:?}"),
        }
    }

    /// Verifies the position-gap design: an `add_text` per attachment
    /// (with boundary padding for slop>=2 defense) prevents a phrase
    /// query whose two halves straddle two attachments from matching.
    ///
    /// This is the highest-impact 7-3 verification step. If Tantivy
    /// future-version changes the per-value position-increment OR the
    /// query parser starts defaulting to non-zero slop, this test
    /// fails and the boundary padding constant needs revisiting.
    #[test]
    fn phase_7_3_attachment_boundary_blocks_cross_attachment_phrase() {
        use tantivy::Index;
        use tantivy::query::QueryParser;
        use tantivy::collector::Count;

        let schema = build_schema();
        let fields = Fields::from_schema(&schema);
        let index = Index::create_in_ram(schema.clone());
        let mut writer: tantivy::IndexWriter = index
            .writer(15_000_000)
            .expect("writer");

        let doc = SearchDocument {
            message_id: "msg1".into(),
            account_id: "acct1".into(),
            thread_id: "t1".into(),
            attachments: vec![
                AttachmentDocFragment {
                    attachment_id: "att1".into(),
                    filename: "first.pdf".into(),
                    mime: "application/pdf".into(),
                    extracted_text: "the quick brown".into(),
                },
                AttachmentDocFragment {
                    attachment_id: "att2".into(),
                    filename: "second.pdf".into(),
                    mime: "application/pdf".into(),
                    extracted_text: "fox jumps over".into(),
                },
            ],
            ..Default::default()
        };
        let tantivy_doc = build_search_doc(&fields, &doc);
        writer.add_document(tantivy_doc).expect("add_document");
        writer.commit().expect("commit");

        let reader = index.reader().expect("reader");
        let searcher = reader.searcher();
        let qp = QueryParser::for_index(&index, vec![fields.attachment_text]);

        // Phrase that straddles the boundary: "brown" is the last
        // token of attachment 1, "fox" is the first token of
        // attachment 2. Default slop = 0; with the boundary padding
        // in place, the position distance is ATTACHMENT_BOUNDARY_REPEATS
        // tokens + tantivy's own POSITION_GAP, well beyond slop=0.
        let cross_query = qp.parse_query("\"brown fox\"").expect("parse cross");
        let cross_count = searcher
            .search(&cross_query, &Count)
            .expect("search cross");
        assert_eq!(
            cross_count, 0,
            "cross-attachment phrase query must NOT match (boundary padding broken?)",
        );

        // Within-attachment phrase still matches.
        let within_query = qp.parse_query("\"the quick\"").expect("parse within");
        let within_count = searcher
            .search(&within_query, &Count)
            .expect("search within");
        assert_eq!(
            within_count, 1,
            "within-attachment phrase query must still match",
        );

        // Single-token from each attachment matches independently.
        let token_query = qp.parse_query("brown").expect("parse token");
        let token_count = searcher.search(&token_query, &Count).expect("search token");
        assert_eq!(token_count, 1, "single-token query must still hit");
    }
}
