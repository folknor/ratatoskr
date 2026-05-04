use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{
    DateOptions, Field, NumericOptions, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::{DateTime as TantivyDateTime, Index, IndexReader, ReloadPolicy, Term};

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

    builder.build()
}

// ── Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub before: Option<i64>,
    pub after: Option<i64>,
    pub limit: Option<usize>,
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
        }
    }
}

// ── Index opener ────────────────────────────────────────────────────────

/// Open or create the Tantivy index in `{app_data_dir}/search_index/`.
///
/// Phase 3 task 4 exposes this so the Service writer task and the UI's
/// `SearchReadState` can both consume the same opener (the writer task
/// runs in `BootPhase::OpeningSearchIndex` and creates the directory if
/// it does not exist; the read state opens after the boot handshake
/// completes).
pub fn open_or_create_search_index(app_data_dir: &Path) -> Result<(Index, Schema), String> {
    let index_dir = app_data_dir.join("search_index");
    std::fs::create_dir_all(&index_dir).map_err(|e| format!("create search index dir: {e}"))?;

    let schema = build_schema();

    log::info!("Opening search index at {}", index_dir.display());

    let index = if Index::exists(
        &tantivy::directory::MmapDirectory::open(&index_dir)
            .map_err(|e| format!("open mmap dir: {e}"))?,
    )
    .map_err(|e| format!("check index exists: {e}"))?
    {
        log::info!("Opening existing search index");
        Index::open_in_dir(&index_dir).map_err(|e| {
            log::error!("Failed to open search index: {e}");
            format!("open index: {e}")
        })?
    } else {
        log::info!("Creating new search index");
        Index::create_in_dir(&index_dir, schema.clone()).map_err(|e| {
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

    doc
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

        // Free text → QueryParser on subject+from_name+body_text+snippet
        if let Some(ref text) = params.free_text
            && !text.is_empty()
        {
            let qp = QueryParser::for_index(
                searcher.index(),
                vec![
                    self.fields.subject,
                    self.fields.from_name,
                    self.fields.body_text,
                    self.fields.snippet,
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

        // Date range: after <= date <= before
        if params.after.is_some() || params.before.is_some() {
            let lower = params
                .after
                .map(|ts| {
                    std::ops::Bound::Included(Term::from_field_date(
                        self.fields.date,
                        TantivyDateTime::from_timestamp_secs(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            let upper = params
                .before
                .map(|ts| {
                    std::ops::Bound::Included(Term::from_field_date(
                        self.fields.date,
                        TantivyDateTime::from_timestamp_secs(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
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
            });
        }

        Ok(results)
    }
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

    // ── Multi-account search tests ───────────────────────────────────
    //
    // The pre-Phase-3 read+write tests lived on `SearchState`; Phase 3
    // task 4 strips writer ownership from `SearchReadState` so the
    // search-with-data tests now live alongside the writer task in
    // `crates/service/src/search_writer.rs`.
}
