use std::future::Future;
use std::pin::Pin;

use bifrost_sync::{CheckpointStore, Error, decode_envelope, encode_envelope};
use bifrost_types::{
    AccountId, BackfillCheckpoint, ChangeCursor, Checkpoint, CursorScope, ObjectType,
};
use db::db::{ReadDbState, WriterPool, params};

pub(crate) const BACKFILL_COMPLETION_PARTITION: &[u8] = b"complete";

#[derive(Clone)]
pub struct SqliteCheckpointStore {
    writer: WriterPool,
    reader: ReadDbState,
}

impl SqliteCheckpointStore {
    pub fn new(writer: WriterPool, reader: ReadDbState) -> Self {
        Self { writer, reader }
    }
}

impl CheckpointStore for SqliteCheckpointStore {
    fn put_change_cursor<'a>(
        &'a self,
        account: &'a AccountId,
        cursor: ChangeCursor,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let scope_key = scope_to_key(&cursor.scope)?;
            let blob = encode_envelope(&Checkpoint::Change(cursor));
            upsert_checkpoint(
                self.writer.clone(),
                account.0.clone(),
                "change",
                scope_key,
                Vec::new(),
                0,
                blob,
            )
            .await
        })
    }

    fn get_change_cursor<'a>(
        &'a self,
        account: &'a AccountId,
        scope: &'a CursorScope,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ChangeCursor>, Error>> + Send + 'a>> {
        Box::pin(async move {
            let scope_key = scope_to_key(scope)?;
            let blob =
                select_change_blob(self.reader.clone(), account.0.clone(), scope_key).await?;
            let Some(blob) = blob else {
                return Ok(None);
            };
            match decode_envelope(&blob)? {
                Checkpoint::Change(cursor) => Ok(Some(cursor)),
                Checkpoint::Backfill(_) => Err(Error::CheckpointStore(
                    "sync_cursors change row decoded as backfill checkpoint".to_string(),
                )),
                _ => Err(Error::CheckpointStore(
                    "sync_cursors change row decoded as unknown checkpoint".to_string(),
                )),
            }
        })
    }

    fn put_backfill<'a>(
        &'a self,
        account: &'a AccountId,
        checkpoint: BackfillCheckpoint,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let scope_key = scope_to_key(&checkpoint.scope)?;
            let partition_key = checkpoint.partition.0.clone();
            let items_done = i64::try_from(checkpoint.progress.items_done).map_err(|_| {
                Error::CheckpointStore(format!(
                    "backfill items_done {} exceeds sqlite integer range",
                    checkpoint.progress.items_done
                ))
            })?;
            let blob = encode_envelope(&Checkpoint::Backfill(checkpoint));
            upsert_checkpoint(
                self.writer.clone(),
                account.0.clone(),
                "backfill",
                scope_key,
                partition_key,
                items_done,
                blob,
            )
            .await
        })
    }

    fn get_backfill<'a>(
        &'a self,
        account: &'a AccountId,
        scope: &'a CursorScope,
    ) -> Pin<Box<dyn Future<Output = Result<Option<BackfillCheckpoint>, Error>> + Send + 'a>> {
        Box::pin(async move {
            let scope_key = scope_to_key(scope)?;
            let blob =
                select_latest_backfill_blob(self.reader.clone(), account.0.clone(), scope_key)
                    .await?;
            let Some(blob) = blob else {
                return Ok(None);
            };
            match decode_envelope(&blob)? {
                Checkpoint::Backfill(checkpoint) => Ok(Some(checkpoint)),
                Checkpoint::Change(_) => Err(Error::CheckpointStore(
                    "sync_cursors backfill row decoded as change cursor".to_string(),
                )),
                _ => Err(Error::CheckpointStore(
                    "sync_cursors backfill row decoded as unknown checkpoint".to_string(),
                )),
            }
        })
    }

    fn delete_change_cursor<'a>(
        &'a self,
        account: &'a AccountId,
        scope: &'a CursorScope,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let scope_key = scope_to_key(scope)?;
            let account_id = account.0.clone();
            self.writer
                .with_write_mapped(
                    move |conn| {
                        conn.execute(
                            "DELETE FROM sync_cursors
                             WHERE account_id = ?1 AND kind = 'change' AND scope_key = ?2",
                            params![account_id, scope_key],
                        )
                        .map_err(|error| Error::CheckpointStore(error.to_string()))?;
                        Ok(())
                    },
                    Error::CheckpointStore,
                )
                .await
        })
    }
}

async fn upsert_checkpoint(
    writer: WriterPool,
    account_id: String,
    kind: &'static str,
    scope_key: String,
    partition_key: Vec<u8>,
    items_done: i64,
    checkpoint_blob: Vec<u8>,
) -> Result<(), Error> {
    writer
        .with_write_mapped(
            move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO sync_cursors (
                        account_id, kind, scope_key, partition_key, items_done,
                        checkpoint_blob, updated_at
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, strftime('%s', 'now')
                     )",
                    params![
                        account_id,
                        kind,
                        scope_key,
                        partition_key,
                        items_done,
                        checkpoint_blob
                    ],
                )
                .map_err(|error| Error::CheckpointStore(error.to_string()))?;
                Ok(())
            },
            Error::CheckpointStore,
        )
        .await
}

async fn select_change_blob(
    reader: ReadDbState,
    account_id: String,
    scope_key: String,
) -> Result<Option<Vec<u8>>, Error> {
    reader
        .with_read_mapped(
            move |conn| match conn.query_row(
                "SELECT checkpoint_blob FROM sync_cursors
                     WHERE account_id = ?1 AND kind = 'change' AND scope_key = ?2",
                params![account_id, scope_key],
                |row| row.get::<_, Vec<u8>>(0),
            ) {
                Ok(blob) => Ok(Some(blob)),
                Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
                Err(error) => Err(Error::CheckpointStore(error.to_string())),
            },
            Error::CheckpointStore,
        )
        .await
}

async fn select_latest_backfill_blob(
    reader: ReadDbState,
    account_id: String,
    scope_key: String,
) -> Result<Option<Vec<u8>>, Error> {
    reader
        .with_read_mapped(
            move |conn| match conn.query_row(
                "SELECT checkpoint_blob FROM sync_cursors
                     WHERE account_id = ?1 AND kind = 'backfill' AND scope_key = ?2
                     ORDER BY
                         CASE WHEN partition_key = ?3 THEN 1 ELSE 0 END DESC,
                         items_done DESC
                     LIMIT 1",
                params![account_id, scope_key, BACKFILL_COMPLETION_PARTITION],
                |row| row.get::<_, Vec<u8>>(0),
            ) {
                Ok(blob) => Ok(Some(blob)),
                Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
                Err(error) => Err(Error::CheckpointStore(error.to_string())),
            },
            Error::CheckpointStore,
        )
        .await
}

fn scope_to_key(scope: &CursorScope) -> Result<String, Error> {
    match scope {
        CursorScope::Account => Ok("account".to_string()),
        CursorScope::Type(ty) => Ok(format!("type:{}", object_type_to_key(*ty)?)),
        CursorScope::Query(query) => Ok(format!("query:{}", encode_key_string(&query.0))),
        CursorScope::Folder(folder) => Ok(format!("folder:{}", encode_key_string(&folder.0))),
        CursorScope::FolderType { folder, ty } => Ok(format!(
            "foldertype:{}:{}",
            encode_key_string(&folder.0),
            object_type_to_key(*ty)?
        )),
        _ => Err(Error::CheckpointStore(
            "unknown cursor scope variant".to_string(),
        )),
    }
}

#[allow(dead_code)]
fn scope_from_key(key: &str) -> Result<CursorScope, Error> {
    if key == "account" {
        return Ok(CursorScope::Account);
    }
    if let Some(value) = key.strip_prefix("type:") {
        return Ok(CursorScope::Type(object_type_from_key(value)?));
    }
    if let Some(value) = key.strip_prefix("query:") {
        return Ok(CursorScope::Query(bifrost_types::QueryId(
            decode_key_string(value)?,
        )));
    }
    if let Some(value) = key.strip_prefix("folder:") {
        return Ok(CursorScope::Folder(bifrost_types::FolderId(
            decode_key_string(value)?,
        )));
    }
    if let Some(value) = key.strip_prefix("foldertype:") {
        let (folder, rest) = decode_key_string_with_rest(value)?;
        let ty = rest.strip_prefix(':').ok_or_else(|| {
            Error::CheckpointStore("foldertype scope key missing object type".to_string())
        })?;
        return Ok(CursorScope::FolderType {
            folder: bifrost_types::FolderId(folder),
            ty: object_type_from_key(ty)?,
        });
    }
    Err(Error::CheckpointStore(format!(
        "unknown cursor scope tag: {key}"
    )))
}

fn encode_key_string(value: &str) -> String {
    format!("{}:{value}", value.len())
}

#[allow(dead_code)]
fn decode_key_string(value: &str) -> Result<String, Error> {
    let (decoded, rest) = decode_key_string_with_rest(value)?;
    if rest.is_empty() {
        Ok(decoded)
    } else {
        Err(Error::CheckpointStore(format!(
            "scope key has trailing data: {rest}"
        )))
    }
}

#[allow(dead_code)]
fn decode_key_string_with_rest(value: &str) -> Result<(String, &str), Error> {
    let Some((len, tail)) = value.split_once(':') else {
        return Err(Error::CheckpointStore(
            "scope key string missing length delimiter".to_string(),
        ));
    };
    let len: usize = len.parse().map_err(|error| {
        Error::CheckpointStore(format!("scope key string has invalid length: {error}"))
    })?;
    if tail.len() < len {
        return Err(Error::CheckpointStore(
            "scope key string shorter than declared length".to_string(),
        ));
    }
    if !tail.is_char_boundary(len) {
        return Err(Error::CheckpointStore(
            "scope key string length splits a character".to_string(),
        ));
    }
    Ok((tail[..len].to_string(), &tail[len..]))
}

fn object_type_to_key(ty: ObjectType) -> Result<&'static str, Error> {
    match ty {
        ObjectType::Email => Ok("email"),
        ObjectType::Mailbox => Ok("mailbox"),
        ObjectType::Thread => Ok("thread"),
        ObjectType::Event => Ok("event"),
        ObjectType::Contact => Ok("contact"),
        ObjectType::EmailSubmission => Ok("email-submission"),
        ObjectType::CalendarEvent => Ok("calendar-event"),
        ObjectType::ContactGroup => Ok("contact-group"),
        _ => Err(Error::CheckpointStore(
            "unknown object type variant".to_string(),
        )),
    }
}

#[allow(dead_code)]
fn object_type_from_key(key: &str) -> Result<ObjectType, Error> {
    match key {
        "email" => Ok(ObjectType::Email),
        "mailbox" => Ok(ObjectType::Mailbox),
        "thread" => Ok(ObjectType::Thread),
        "event" => Ok(ObjectType::Event),
        "contact" => Ok(ObjectType::Contact),
        "email-submission" => Ok(ObjectType::EmailSubmission),
        "calendar-event" => Ok(ObjectType::CalendarEvent),
        "contact-group" => Ok(ObjectType::ContactGroup),
        _ => Err(Error::CheckpointStore(format!(
            "unknown object type tag: {key}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use bifrost_sync::InMemoryCheckpointStore;
    use bifrost_types::{
        BackfillProgress, FolderId, OpaqueChangeState, OpaqueProgressBytes, Partition,
        ProtocolKind, QueryId,
    };
    use db::db::{open_reader_pool, open_writer_pool};

    use super::*;

    #[test]
    fn scope_key_roundtrips() {
        let scopes = all_constructed_scopes();
        let mut keys = HashSet::new();
        for scope in &scopes {
            let key = scope_to_key(scope).expect("scope to key");
            let decoded = scope_from_key(&key).expect("scope from key");
            assert_eq!(decoded, *scope);
            assert!(keys.insert(key), "scope key collision for {scope:?}");
        }

        let err = scope_from_key("future:shape").expect_err("unknown tag should fail");
        assert!(matches!(err, Error::CheckpointStore(_)));
    }

    #[tokio::test]
    async fn checkpoint_store_change_roundtrip() {
        let (writer, reader, dir) = test_dbs("change");
        seed_account(&writer, "acct-a").await;
        let store = SqliteCheckpointStore::new(writer.clone(), reader);
        let account = AccountId("acct-a".to_string());
        let scope = CursorScope::Type(ObjectType::Email);

        assert_eq!(
            store
                .get_change_cursor(&account, &scope)
                .await
                .expect("empty get"),
            None
        );

        let first = change_cursor(scope.clone(), ProtocolKind::Jmap, 7, Some(vec![1, 2, 3]));
        store
            .put_change_cursor(&account, first.clone())
            .await
            .expect("put first");
        assert_eq!(
            store
                .get_change_cursor(&account, &scope)
                .await
                .expect("get first"),
            Some(first.clone())
        );

        let replacement = change_cursor(scope.clone(), ProtocolKind::Graph, 11, None);
        store
            .put_change_cursor(&account, replacement.clone())
            .await
            .expect("put replacement");
        assert_eq!(
            store
                .get_change_cursor(&account, &scope)
                .await
                .expect("get replacement"),
            Some(replacement.clone())
        );
        assert_eq!(count_rows(&writer, "change", "acct-a").await, 1);

        store
            .delete_change_cursor(&account, &scope)
            .await
            .expect("delete cursor");
        assert_eq!(
            store
                .get_change_cursor(&account, &scope)
                .await
                .expect("get after delete"),
            None
        );

        let missing_account = AccountId("missing".to_string());
        let err = store
            .put_change_cursor(&missing_account, replacement)
            .await
            .expect_err("missing account should fail FK");
        assert!(matches!(err, Error::CheckpointStore(_)));

        remove_test_dir(dir);
    }

    #[tokio::test]
    async fn checkpoint_store_backfill_roundtrip() {
        let (writer, reader, dir) = test_dbs("backfill");
        seed_account(&writer, "acct-a").await;
        let store = SqliteCheckpointStore::new(writer.clone(), reader);
        let account = AccountId("acct-a".to_string());
        let scope = CursorScope::Folder(FolderId("folder-a".to_string()));

        assert!(
            store
                .get_backfill(&account, &scope)
                .await
                .expect("empty get")
                .is_none()
        );

        let first = backfill_checkpoint(scope.clone(), b"A".to_vec(), 10, Some(vec![9]));
        store
            .put_backfill(&account, first.clone())
            .await
            .expect("put first");
        assert_backfill_eq(
            &store
                .get_backfill(&account, &scope)
                .await
                .expect("get first")
                .expect("first checkpoint"),
            &first,
        );

        let replacement = backfill_checkpoint(scope.clone(), b"A".to_vec(), 12, None);
        store
            .put_backfill(&account, replacement.clone())
            .await
            .expect("replace same partition");
        assert_eq!(count_rows(&writer, "backfill", "acct-a").await, 1);
        assert_backfill_eq(
            &store
                .get_backfill(&account, &scope)
                .await
                .expect("get replacement")
                .expect("replacement checkpoint"),
            &replacement,
        );

        let later_insert_lower_progress =
            backfill_checkpoint(scope.clone(), b"B".to_vec(), 3, Some(vec![1, 2]));
        store
            .put_backfill(&account, later_insert_lower_progress)
            .await
            .expect("put second partition");
        assert_eq!(count_rows(&writer, "backfill", "acct-a").await, 2);
        assert_backfill_eq(
            &store
                .get_backfill(&account, &scope)
                .await
                .expect("get latest")
                .expect("latest checkpoint"),
            &replacement,
        );

        let complete_lower_progress = backfill_checkpoint(
            scope.clone(),
            BACKFILL_COMPLETION_PARTITION.to_vec(),
            1,
            None,
        );
        store
            .put_backfill(&account, complete_lower_progress.clone())
            .await
            .expect("put completion marker");
        assert_eq!(count_rows(&writer, "backfill", "acct-a").await, 3);
        assert_backfill_eq(
            &store
                .get_backfill(&account, &scope)
                .await
                .expect("get completion marker")
                .expect("completion checkpoint"),
            &complete_lower_progress,
        );

        remove_test_dir(dir);
    }

    #[tokio::test]
    async fn checkpoint_store_matches_in_memory() {
        let (writer, reader, dir) = test_dbs("parity");
        seed_account(&writer, "acct-a").await;
        seed_account(&writer, "acct-b").await;
        let sqlite =
            Arc::new(SqliteCheckpointStore::new(writer, reader)) as Arc<dyn CheckpointStore>;
        let memory = Arc::new(InMemoryCheckpointStore::new()) as Arc<dyn CheckpointStore>;

        run_checkpoint_script(Arc::clone(&sqlite)).await;
        run_checkpoint_script(Arc::clone(&memory)).await;

        let accounts = [
            AccountId("acct-a".to_string()),
            AccountId("acct-b".to_string()),
        ];
        let scopes = [
            CursorScope::Account,
            CursorScope::Type(ObjectType::Email),
            CursorScope::Folder(FolderId("folder-a".to_string())),
        ];
        for account in &accounts {
            for scope in &scopes {
                assert_eq!(
                    sqlite
                        .get_change_cursor(account, scope)
                        .await
                        .expect("sqlite change get"),
                    memory
                        .get_change_cursor(account, scope)
                        .await
                        .expect("memory change get"),
                );
                assert_backfill_options_eq(
                    sqlite
                        .get_backfill(account, scope)
                        .await
                        .expect("sqlite backfill get"),
                    memory
                        .get_backfill(account, scope)
                        .await
                        .expect("memory backfill get"),
                );
            }
        }

        remove_test_dir(dir);
    }

    async fn run_checkpoint_script(store: Arc<dyn CheckpointStore>) {
        let acct_a = AccountId("acct-a".to_string());
        let acct_b = AccountId("acct-b".to_string());
        let account_scope = CursorScope::Account;
        let email_scope = CursorScope::Type(ObjectType::Email);
        let folder_scope = CursorScope::Folder(FolderId("folder-a".to_string()));

        store
            .put_change_cursor(
                &acct_a,
                change_cursor(account_scope.clone(), ProtocolKind::Gmail, 1, None),
            )
            .await
            .expect("put acct a account change");
        store
            .put_change_cursor(
                &acct_b,
                change_cursor(account_scope.clone(), ProtocolKind::Gmail, 2, Some(vec![8])),
            )
            .await
            .expect("put acct b account change");
        store
            .put_change_cursor(
                &acct_a,
                change_cursor(email_scope.clone(), ProtocolKind::Jmap, 3, Some(vec![1])),
            )
            .await
            .expect("put acct a email change");
        store
            .put_change_cursor(
                &acct_a,
                change_cursor(email_scope.clone(), ProtocolKind::Jmap, 4, None),
            )
            .await
            .expect("replace acct a email change");
        store
            .delete_change_cursor(&acct_a, &email_scope)
            .await
            .expect("delete acct a email change");

        store
            .put_backfill(
                &acct_a,
                backfill_checkpoint(folder_scope.clone(), b"A".to_vec(), 10, None),
            )
            .await
            .expect("put partition A");
        store
            .put_backfill(
                &acct_a,
                backfill_checkpoint(folder_scope.clone(), b"B".to_vec(), 3, Some(vec![3])),
            )
            .await
            .expect("put partition B");
        store
            .put_backfill(
                &acct_b,
                backfill_checkpoint(folder_scope, b"A".to_vec(), 20, Some(vec![2])),
            )
            .await
            .expect("put acct b partition");
    }

    fn all_constructed_scopes() -> Vec<CursorScope> {
        let mut scopes = vec![CursorScope::Account];
        for ty in all_object_types() {
            scopes.push(CursorScope::Type(ty));
        }
        scopes.push(CursorScope::Query(QueryId(
            "query:has\x1fdelims".to_string(),
        )));
        scopes.push(CursorScope::Folder(FolderId(
            "folder:has\x1fdelims".to_string(),
        )));
        scopes.push(CursorScope::FolderType {
            folder: FolderId("folder:type:\x1fcompound".to_string()),
            ty: ObjectType::CalendarEvent,
        });
        scopes
    }

    fn all_object_types() -> Vec<ObjectType> {
        vec![
            ObjectType::Email,
            ObjectType::Mailbox,
            ObjectType::Thread,
            ObjectType::Event,
            ObjectType::Contact,
            ObjectType::EmailSubmission,
            ObjectType::CalendarEvent,
            ObjectType::ContactGroup,
        ]
    }

    fn change_cursor(
        scope: CursorScope,
        protocol: ProtocolKind,
        marker: u8,
        advanced: Option<Vec<u8>>,
    ) -> ChangeCursor {
        ChangeCursor {
            scope,
            server_state: OpaqueChangeState {
                protocol,
                envelope_version: u32::from(marker) + 100,
                bytes: vec![marker, marker.saturating_add(1)],
            },
            advanced_through: advanced.map(OpaqueProgressBytes),
            envelope_version: 1,
        }
    }

    fn backfill_checkpoint(
        scope: CursorScope,
        partition: Vec<u8>,
        items_done: u64,
        marker: Option<Vec<u8>>,
    ) -> BackfillCheckpoint {
        BackfillCheckpoint {
            scope,
            partition: Partition(partition),
            progress_marker: marker.map(OpaqueProgressBytes),
            progress: BackfillProgress {
                items_done,
                items_estimated: Some(items_done.saturating_add(100)),
            },
            envelope_version: 1,
        }
    }

    fn assert_backfill_options_eq(
        left: Option<BackfillCheckpoint>,
        right: Option<BackfillCheckpoint>,
    ) {
        match (left, right) {
            (None, None) => {}
            (Some(left), Some(right)) => assert_backfill_eq(&left, &right),
            (left, right) => panic!("backfill mismatch: {left:?} != {right:?}"),
        }
    }

    fn assert_backfill_eq(left: &BackfillCheckpoint, right: &BackfillCheckpoint) {
        assert_eq!(left.scope, right.scope);
        assert_eq!(left.partition, right.partition);
        assert_eq!(left.progress_marker, right.progress_marker);
        assert_eq!(left.progress.items_done, right.progress.items_done);
        assert_eq!(
            left.progress.items_estimated,
            right.progress.items_estimated
        );
        assert_eq!(left.envelope_version, right.envelope_version);
    }

    fn test_dbs(name: &str) -> (WriterPool, ReadDbState, std::path::PathBuf) {
        let dir = std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join("bifrost-checkpoint-store-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        let writer = open_writer_pool(&dir).expect("open writer pool");
        let reader = open_reader_pool(&dir).expect("open reader pool");
        (writer, reader, dir)
    }

    async fn seed_account(writer: &WriterPool, id: &str) {
        writer
            .with_write({
                let id = id.to_string();
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, provider, auth_method, account_name, account_color
                         ) VALUES (?1, ?2, 'jmap', 'oauth2', 'Test', '#000000')",
                        params![id, format!("{id}@example.test")],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed account");
    }

    async fn count_rows(writer: &WriterPool, kind: &str, account_id: &str) -> i64 {
        let kind = kind.to_string();
        let account_id = account_id.to_string();
        writer
            .with_read(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM sync_cursors
                     WHERE kind = ?1 AND account_id = ?2",
                    params![kind, account_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())
            })
            .await
            .expect("count rows")
    }

    fn remove_test_dir(dir: std::path::PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
