use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{
    DateOptions, Field, NumericOptions, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::{DateTime as TantivyDateTime, Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use tokio::sync::Mutex;

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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
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
    pub account_id: String,
    pub free_text: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: Option<bool>,
    pub is_unread: Option<bool>,
    pub is_starred: Option<bool>,
    pub before: Option<i64>,
    pub after: Option<i64>,
    /// Label filter — not handled in tantivy; caller must post-filter.
    #[allow(dead_code)]
    pub label: Option<String>,
    pub limit: Option<usize>,
}

// ── Field helpers ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Fields {
    subject: Field,
    from_name: Field,
    from_address: Field,
    to_addresses: Field,
    body_text: Field,
    snippet: Field,
    message_id: Field,
    thread_id: Field,
    account_id: Field,
    date: Field,
    is_read: Field,
    is_starred: Field,
    has_attachment: Field,
}

impl Fields {
    fn from_schema(schema: &Schema) -> Self {
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

// ── SearchState ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SearchState {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<IndexWriter>>,
    #[allow(dead_code)]
    schema: Schema,
    fields: Fields,
}

// Safety: Index, IndexReader are Send+Sync; writer is behind Arc<Mutex>
unsafe impl Send for SearchState {}
unsafe impl Sync for SearchState {}

const WRITER_HEAP_BYTES: usize = 50_000_000; // 50 MB

impl SearchState {
    /// Open or create the tantivy index in `{app_data_dir}/search_index/`.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        let index_dir = app_data_dir.join("search_index");
        std::fs::create_dir_all(&index_dir).map_err(|e| format!("create search index dir: {e}"))?;

        let schema = build_schema();

        let index = if Index::exists(
            &tantivy::directory::MmapDirectory::open(&index_dir)
                .map_err(|e| format!("open mmap dir: {e}"))?,
        )
        .map_err(|e| format!("check index exists: {e}"))?
        {
            Index::open_in_dir(&index_dir).map_err(|e| format!("open index: {e}"))?
        } else {
            Index::create_in_dir(&index_dir, schema.clone())
                .map_err(|e| format!("create index: {e}"))?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("create reader: {e}"))?;

        let writer = index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| format!("create writer: {e}"))?;

        let fields = Fields::from_schema(&schema);

        Ok(Self {
            index,
            reader,
            writer: Arc::new(Mutex::new(writer)),
            schema,
            fields,
        })
    }

    /// Convert a unix timestamp (seconds) to tantivy DateTime.
    fn to_tantivy_date(ts: i64) -> TantivyDateTime {
        TantivyDateTime::from_timestamp_secs(ts)
    }

    /// Build a tantivy document from a `SearchDocument`.
    fn build_doc(&self, msg: &SearchDocument) -> tantivy::TantivyDocument {
        let mut doc = tantivy::TantivyDocument::default();

        doc.add_text(self.fields.message_id, &msg.message_id);
        doc.add_text(self.fields.account_id, &msg.account_id);
        doc.add_text(self.fields.thread_id, &msg.thread_id);
        doc.add_text(
            self.fields.subject,
            msg.subject.as_deref().unwrap_or_default(),
        );
        doc.add_text(
            self.fields.from_name,
            msg.from_name.as_deref().unwrap_or_default(),
        );
        doc.add_text(
            self.fields.from_address,
            msg.from_address.as_deref().unwrap_or_default(),
        );
        doc.add_text(
            self.fields.to_addresses,
            msg.to_addresses.as_deref().unwrap_or_default(),
        );
        doc.add_text(
            self.fields.body_text,
            msg.body_text.as_deref().unwrap_or_default(),
        );
        doc.add_text(
            self.fields.snippet,
            msg.snippet.as_deref().unwrap_or_default(),
        );
        doc.add_date(self.fields.date, Self::to_tantivy_date(msg.date));
        doc.add_u64(self.fields.is_read, u64::from(msg.is_read));
        doc.add_u64(self.fields.is_starred, u64::from(msg.is_starred));
        doc.add_u64(self.fields.has_attachment, u64::from(msg.has_attachment));

        doc
    }

    /// Index a single message. Commits immediately.
    pub async fn index_message(&self, msg: &SearchDocument) -> Result<(), String> {
        let doc = self.build_doc(msg);
        let mut writer = self.writer.lock().await;

        // Delete any existing doc with same message_id to avoid duplicates
        writer.delete_term(Term::from_field_text(
            self.fields.message_id,
            &msg.message_id,
        ));
        writer
            .add_document(doc)
            .map_err(|e| format!("add document: {e}"))?;
        writer.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    /// Batch-index multiple messages. Single commit at the end.
    pub async fn index_messages_batch(&self, msgs: &[SearchDocument]) -> Result<(), String> {
        let mut writer = self.writer.lock().await;

        for msg in msgs {
            writer.delete_term(Term::from_field_text(
                self.fields.message_id,
                &msg.message_id,
            ));
            let doc = self.build_doc(msg);
            writer
                .add_document(doc)
                .map_err(|e| format!("add document: {e}"))?;
        }

        writer.commit().map_err(|e| format!("commit batch: {e}"))?;
        Ok(())
    }

    /// Delete a document by message_id.
    pub async fn delete_message(&self, message_id: &str) -> Result<(), String> {
        let mut writer = self.writer.lock().await;
        writer.delete_term(Term::from_field_text(self.fields.message_id, message_id));
        writer.commit().map_err(|e| format!("commit delete: {e}"))?;
        Ok(())
    }

    /// Simple free-text search filtered by account_id.
    #[allow(dead_code)]
    pub fn search(
        &self,
        query_str: &str,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            searcher.index(),
            vec![
                self.fields.subject,
                self.fields.from_name,
                self.fields.body_text,
                self.fields.snippet,
            ],
        );

        let text_query = query_parser
            .parse_query(query_str)
            .map_err(|e| format!("parse query: {e}"))?;

        let account_filter: Box<dyn Query> = Box::new(TermQuery::new(
            Term::from_field_text(self.fields.account_id, account_id),
            tantivy::schema::IndexRecordOption::Basic,
        ));

        let combined = BooleanQuery::new(vec![
            (Occur::Must, text_query),
            (Occur::Must, account_filter),
        ]);

        let top_docs = searcher
            .search(&combined, &TopDocs::with_limit(limit))
            .map_err(|e| format!("search: {e}"))?;

        self.collect_results(&searcher, &top_docs)
    }

    /// Advanced search with structured filters.
    #[allow(clippy::too_many_lines)]
    pub fn search_with_filters(&self, params: &SearchParams) -> Result<Vec<SearchResult>, String> {
        let searcher = self.reader.searcher();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Always filter by account_id
        clauses.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(self.fields.account_id, &params.account_id),
                tantivy::schema::IndexRecordOption::Basic,
            )),
        ));

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

        // from → TermQuery on from_address OR phrase on from_name
        if let Some(ref from) = params.from
            && !from.is_empty()
        {
            let from_addr: Box<dyn Query> = Box::new(TermQuery::new(
                Term::from_field_text(self.fields.from_address, from),
                tantivy::schema::IndexRecordOption::Basic,
            ));
            let from_name_qp =
                QueryParser::for_index(searcher.index(), vec![self.fields.from_name]);
            let from_name = from_name_qp
                .parse_query(from)
                .map_err(|e| format!("parse from: {e}"))?;
            clauses.push((
                Occur::Must,
                Box::new(BooleanQuery::new(vec![
                    (Occur::Should, from_addr),
                    (Occur::Should, from_name),
                ])),
            ));
        }

        // to → phrase on to_addresses
        if let Some(ref to) = params.to
            && !to.is_empty()
        {
            let to_qp = QueryParser::for_index(searcher.index(), vec![self.fields.to_addresses]);
            let q = to_qp
                .parse_query(to)
                .map_err(|e| format!("parse to: {e}"))?;
            clauses.push((Occur::Must, q));
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
                        Self::to_tantivy_date(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            let upper = params
                .before
                .map(|ts| {
                    std::ops::Bound::Included(Term::from_field_date(
                        self.fields.date,
                        Self::to_tantivy_date(ts),
                    ))
                })
                .unwrap_or(std::ops::Bound::Unbounded);
            clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
        }

        // label filter is not supported in tantivy (lives in SQLite); caller post-filters.

        let limit = params.limit.unwrap_or(50);

        if clauses.is_empty() {
            return Ok(Vec::new());
        }

        let combined = BooleanQuery::new(clauses);
        let top_docs = searcher
            .search(&combined, &TopDocs::with_limit(limit))
            .map_err(|e| format!("search: {e}"))?;

        self.collect_results(&searcher, &top_docs)
    }

    /// Clear all documents from the index.
    pub async fn clear_index(&self) -> Result<(), String> {
        let mut writer = self.writer.lock().await;
        writer
            .delete_all_documents()
            .map_err(|e| format!("delete all: {e}"))?;
        writer.commit().map_err(|e| format!("commit clear: {e}"))?;
        Ok(())
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
