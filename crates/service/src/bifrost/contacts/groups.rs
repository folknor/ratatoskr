//! Provider-agnostic directory-group (Exchange DL / M365 group) snapshot
//! pull over the bifrost `directory_groups_list` / `directory_group_expand`
//! surface. Sibling of `pull.rs`, and the replacement for the deleted
//! `graph::group_sync`.

use std::collections::HashSet;
use std::future::Future;

use bifrost_sync::{Error, SyncEngine};
use bifrost_types::{AccountId, DirectoryGroup, DirectoryGroupKind, Page};
use db::db::WriteTxn;
use db::db::queries_extra::{
    ContactGroupRow, delete_contact_group_by_id, delete_contact_group_members,
    delete_contact_groups_for_account_by_source, insert_contact_group_member_email,
    list_contact_groups_for_account_by_source, upsert_contact_group,
};
use service_state::WriteDbState;

/// Provider rows keep the pre-cut source label so existing rows, the
/// `(account_id, server_id)` unique index, and every generic
/// `contact_groups` consumer (compose expansion, settings UI) are untouched.
pub const DIRECTORY_GROUP_SOURCE: &str = "exchange";
/// Settings-table cycle key, distinct from the contact pull's
/// `contact_pull_cycle:{account}:{provider}` keys.
pub const DIRECTORY_GROUP_CYCLE_SOURCE: &str = "directory_groups";
pub const DIRECTORY_GROUP_CYCLE_DIVISOR: u32 = 20;

/// Legacy display name for a group whose provider row carried no name.
const UNNAMED_GROUP: &str = "Unnamed Group";

#[must_use]
pub fn should_pull_groups_on_cycle(cycle: u32) -> bool {
    cycle.is_multiple_of(DIRECTORY_GROUP_CYCLE_DIVISOR)
}

/// One enumerated group, post-mapping, pre-persist.
pub(crate) struct PulledGroup {
    pub server_id: String,
    /// `display_name`, with the legacy "Unnamed Group" fallback applied when
    /// the provider row carried an empty name.
    pub name: String,
    pub email: Option<String>,
    /// `"m365"` | `"distribution_list"` | `"mail_security"`.
    pub group_type: &'static str,
    /// Lowercased member emails from the transitive expansion. `None` = the
    /// expansion for THIS group failed transiently; persist keeps the group
    /// row and leaves its existing member rows untouched.
    pub members: Option<Vec<String>>,
}

/// Outcome of one pull, so callers (and the harness ack) can tell a clean
/// delete-all apart from a capability no-op - both would otherwise report
/// zero groups with opposite DB effects.
pub struct GroupPullOutcome {
    /// False when the protocol reported `Unsupported` on the FIRST call:
    /// nothing was written, existing rows are intact.
    pub supported: bool,
    /// Groups in the completed snapshot (0 when `supported` is false).
    pub groups: usize,
}

/// Enumerate + expand through the engine, then persist the snapshot.
pub async fn run_group_pull(
    engine: &SyncEngine,
    account_id: &str,
    write_db: &WriteDbState,
) -> Result<GroupPullOutcome, String> {
    let account = AccountId(account_id.to_string());
    let Some(groups) =
        collect_group_pages(|cursor| engine.directory_groups_list(&account, cursor)).await?
    else {
        return Ok(GroupPullOutcome {
            supported: false,
            groups: 0,
        });
    };

    let mut pulled = Vec::with_capacity(groups.len());
    for group in groups {
        let members = expand_group_members(engine, &account, &group).await;
        pulled.push(PulledGroup {
            server_id: group.id.0,
            name: group_name(group.display_name),
            email: group.email,
            group_type: group_type_str(group.kind),
            members,
        });
    }

    let count = pulled.len();
    let account_id = account_id.to_string();
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("begin group pull: {error}"))?;
            persist_group_snapshot(&tx, &account_id, &pulled)?;
            tx.commit()
                .map_err(|error| format!("commit group pull: {error}"))?;
            Ok(())
        })
        .await?;

    Ok(GroupPullOutcome {
        supported: true,
        groups: count,
    })
}

/// Page the group enumeration to exhaustion.
///
/// `Ok(None)` means the protocol reported `Unsupported` on the FIRST call -
/// a clean capability no-op with zero writes. Every other error, on ANY
/// page, is `Err`: a partial enumeration must never be mistaken for a
/// completed snapshot, because persisting it would prune every group living
/// on the pages that were never reached. An `Unsupported` arriving
/// mid-enumeration is a protocol contradiction and takes the `Err` path.
async fn collect_group_pages<F, Fut>(mut fetch: F) -> Result<Option<Vec<DirectoryGroup>>, String>
where
    F: FnMut(Option<Vec<u8>>) -> Fut,
    Fut: Future<Output = Result<Page<DirectoryGroup>, Error>>,
{
    let mut cursor = None;
    let mut first_call = true;
    let mut groups = Vec::new();
    loop {
        let page = match fetch(cursor).await {
            Ok(page) => page,
            Err(error) if first_call && is_unsupported(&error) => return Ok(None),
            Err(error) => return Err(format!("directory group list: {error}")),
        };
        first_call = false;
        groups.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(Some(groups));
        }
    }
}

/// Page one group's transitive expansion. A failure on any page logs and
/// yields `None` - it never fails the whole pull and never wipes that
/// group's existing member rows.
async fn expand_group_members(
    engine: &SyncEngine,
    account: &AccountId,
    group: &DirectoryGroup,
) -> Option<Vec<String>> {
    let mut cursor = None;
    let mut members = Vec::new();
    loop {
        match engine
            .directory_group_expand(account, group.id.clone(), cursor)
            .await
        {
            Ok(page) => {
                members.extend(page.items.into_iter().map(|member| member.email));
                cursor = page.next_cursor;
                if cursor.is_none() {
                    return Some(members);
                }
            }
            Err(error) => {
                log::warn!(
                    "directory group expansion for {} failed: {error}",
                    group.id.0
                );
                return None;
            }
        }
    }
}

/// Pure persistence half: one transaction doing upserts, member replaces,
/// and the stale prune. Writes no SQL of its own - every statement is a `db`
/// helper.
pub(crate) fn persist_group_snapshot(
    tx: &WriteTxn<'_>,
    account_id: &str,
    pulled: &[PulledGroup],
) -> Result<usize, String> {
    // A clean empty enumeration is a real delete-all, not a no-op.
    if pulled.is_empty() {
        delete_contact_groups_for_account_by_source(tx, account_id, DIRECTORY_GROUP_SOURCE)?;
        return Ok(0);
    }

    let mut seen = HashSet::with_capacity(pulled.len());
    for group in pulled {
        let id = format!("{DIRECTORY_GROUP_SOURCE}-{account_id}-{}", group.server_id);
        upsert_contact_group(
            tx,
            &ContactGroupRow {
                id: id.clone(),
                name: group.name.clone(),
                source: DIRECTORY_GROUP_SOURCE.to_string(),
                account_id: account_id.to_string(),
                server_id: group.server_id.clone(),
                email: group.email.clone(),
                group_type: group.group_type.to_string(),
            },
        )?;
        if let Some(members) = &group.members {
            delete_contact_group_members(tx, &id)?;
            for email in members {
                insert_contact_group_member_email(tx, &id, email)?;
            }
        }
        seen.insert(group.server_id.as_str());
    }

    // Members cascade with the group row via the FK.
    let existing = list_contact_groups_for_account_by_source(
        &tx.as_read(),
        account_id,
        DIRECTORY_GROUP_SOURCE,
    )?;
    for (id, server_id) in existing {
        if !seen.contains(server_id.as_str()) {
            delete_contact_group_by_id(tx, &id)?;
        }
    }

    Ok(pulled.len())
}

fn group_name(display_name: String) -> String {
    if display_name.is_empty() {
        UNNAMED_GROUP.to_string()
    } else {
        display_name
    }
}

fn group_type_str(kind: DirectoryGroupKind) -> &'static str {
    match kind {
        DirectoryGroupKind::Unified => "m365",
        DirectoryGroupKind::MailEnabledSecurity => "mail_security",
        DirectoryGroupKind::DistributionList => "distribution_list",
        // `DirectoryGroupKind` is `#[non_exhaustive]`. An unknown future kind
        // maps to the neutral plain mail-enabled bucket rather than being
        // dropped: dropping would prune the group on the next reconcile and
        // destroy its members.
        _ => "distribution_list",
    }
}

fn is_unsupported(error: &Error) -> bool {
    matches!(
        error,
        Error::Account(account_error)
            if matches!(account_error.recovery(), bifrost_types::RecoveryClass::Unsupported(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_types::ProtocolKind;
    use bifrost_types::directory::DirectoryGroupId;
    use bifrost_types::error::{
        AccessCause, AccessErrorKind, AccountErrorBuilder, AccountErrorKind, AccountOperation,
        Cause, Protocol, RequestCause,
    };
    use db::db::WriteConn;
    use db::db::queries_extra::next_contact_pull_cycle_sync;
    use rusqlite::params;
    use service_state::WriteDbState;
    use tempfile::TempDir;

    const ACCOUNT: &str = "acct-1";

    fn fresh_db() -> (WriteDbState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let pool = db::db::open_writer_pool(tmp.path()).expect("writer pool");
        (WriteDbState::from_pool(pool), tmp)
    }

    fn with_conn<T>(db: &WriteDbState, f: impl FnOnce(&WriteConn<'_>) -> T) -> T {
        db.with_write_sync(|conn| Ok(f(conn))).expect("writer conn")
    }

    fn in_txn<T>(db: &WriteDbState, f: impl FnOnce(&WriteTxn<'_>) -> T) -> T {
        with_conn(db, |conn| {
            let tx = conn.transaction().expect("begin");
            let out = f(&tx);
            tx.commit().expect("commit");
            out
        })
    }

    fn seed_account(conn: &WriteConn<'_>, account_id: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, email) VALUES (?1, ?2)",
            params![account_id, format!("{account_id}@example.test")],
        )
        .expect("seed account");
    }

    fn seed_group(conn: &WriteConn<'_>, account_id: &str, server_id: &str, members: &[&str]) {
        let id = format!("exchange-{account_id}-{server_id}");
        conn.execute(
            "INSERT INTO contact_groups (id, name, source, account_id, server_id, group_type) \
             VALUES (?1, ?2, 'exchange', ?3, ?4, 'distribution_list')",
            params![id, format!("seed-{server_id}"), account_id, server_id],
        )
        .expect("seed group");
        for email in members {
            conn.execute(
                "INSERT INTO contact_group_members (group_id, member_type, member_value) \
                 VALUES (?1, 'email', ?2)",
                params![id, email],
            )
            .expect("seed member");
        }
    }

    fn seed_user_group(conn: &WriteConn<'_>, id: &str) {
        conn.execute(
            "INSERT INTO contact_groups (id, name, source) VALUES (?1, 'Friends', 'user')",
            params![id],
        )
        .expect("seed user group");
    }

    fn group_server_ids(conn: &WriteConn<'_>, account_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT server_id FROM contact_groups \
                 WHERE account_id = ?1 AND source = 'exchange' ORDER BY server_id",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(params![account_id], |row| row.get::<_, String>(0))
            .expect("query");
        rows.map(|row| row.expect("row")).collect()
    }

    fn members_of(conn: &WriteConn<'_>, group_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT member_value FROM contact_group_members \
                 WHERE group_id = ?1 AND member_type = 'email' ORDER BY member_value",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(params![group_id], |row| row.get::<_, String>(0))
            .expect("query");
        rows.map(|row| row.expect("row")).collect()
    }

    fn group_name_of(conn: &WriteConn<'_>, group_id: &str) -> String {
        conn.query_row(
            "SELECT name FROM contact_groups WHERE id = ?1",
            params![group_id],
            |row| row.get(0),
        )
        .expect("group row")
    }

    fn pulled(server_id: &str, members: Option<&[&str]>) -> PulledGroup {
        PulledGroup {
            server_id: server_id.to_string(),
            name: format!("Group {server_id}"),
            email: Some(format!("{server_id}@example.test")),
            group_type: "distribution_list",
            members: members.map(|m| m.iter().map(|email| (*email).to_string()).collect()),
        }
    }

    fn unsupported_error() -> Error {
        let account_error = AccountErrorBuilder::new(
            AccountErrorKind::Unsupported(AccountOperation::DirectoryGroupsList),
            Cause::Request(RequestCause::Unsupported {
                operation: AccountOperation::DirectoryGroupsList,
            }),
        )
        .protocol(Protocol::Jmap)
        .operation(AccountOperation::DirectoryGroupsList)
        .try_build()
        .expect("valid account error");
        Error::Account(account_error)
    }

    /// A Graph-shaped `NoPermission` - the unconsented-tenant case O4 exists
    /// to protect against. It must never be mistaken for a capability no-op.
    fn no_permission_error() -> Error {
        let account_error = AccountErrorBuilder::new(
            AccountErrorKind::Authorization(AccessErrorKind::PermissionDenied),
            Cause::Access(AccessCause::PermissionDenied { resource: None }),
        )
        .protocol(Protocol::Graph)
        .operation(AccountOperation::DirectoryGroupsList)
        .try_build()
        .expect("valid account error");
        Error::Account(account_error)
    }

    fn group(server_id: &str, kind: DirectoryGroupKind) -> DirectoryGroup {
        DirectoryGroup {
            id: DirectoryGroupId(server_id.to_string()),
            display_name: format!("Group {server_id}"),
            email: None,
            kind,
            provider: ProtocolKind::Graph,
        }
    }

    fn page(items: Vec<DirectoryGroup>, next: Option<&str>) -> Page<DirectoryGroup> {
        Page {
            items,
            next_cursor: next.map(|cursor| cursor.as_bytes().to_vec()),
            estimated_total: None,
            failed_ids: Vec::new(),
            skipped_scopes: Vec::new(),
        }
    }

    #[test]
    fn group_type_str_maps_kinds() {
        assert_eq!(group_type_str(DirectoryGroupKind::Unified), "m365");
        assert_eq!(
            group_type_str(DirectoryGroupKind::DistributionList),
            "distribution_list"
        );
        assert_eq!(
            group_type_str(DirectoryGroupKind::MailEnabledSecurity),
            "mail_security"
        );
        // O6: bifrost collapses an absent and an empty display name to "".
        assert_eq!(group_name(String::new()), "Unnamed Group");
        assert_eq!(group_name("Engineering".to_string()), "Engineering");
    }

    #[test]
    fn group_pull_cadence_divisor() {
        assert!(should_pull_groups_on_cycle(0));
        assert!(!should_pull_groups_on_cycle(1));
        assert!(!should_pull_groups_on_cycle(19));
        assert!(should_pull_groups_on_cycle(20));
    }

    #[test]
    fn group_pull_cycle_counter_is_isolated_from_contact_pull() {
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            // O7: advancing the contact counter must not move the group one.
            assert_eq!(
                next_contact_pull_cycle_sync(conn, ACCOUNT, "graph").expect("cycle"),
                0
            );
            assert_eq!(
                next_contact_pull_cycle_sync(conn, ACCOUNT, "graph").expect("cycle"),
                1
            );
            assert_eq!(
                next_contact_pull_cycle_sync(conn, ACCOUNT, DIRECTORY_GROUP_CYCLE_SOURCE)
                    .expect("cycle"),
                0
            );
            assert_eq!(
                next_contact_pull_cycle_sync(conn, ACCOUNT, "graph").expect("cycle"),
                2
            );
            assert_eq!(
                next_contact_pull_cycle_sync(conn, ACCOUNT, DIRECTORY_GROUP_CYCLE_SOURCE)
                    .expect("cycle"),
                1
            );
        });
    }

    #[test]
    fn persist_group_snapshot_upserts_and_replaces_members() {
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            seed_group(conn, ACCOUNT, "g1", &["stale@example.test"]);
        });
        in_txn(&db, |tx| {
            persist_group_snapshot(tx, ACCOUNT, &[pulled("g1", Some(&["fresh@example.test"]))])
                .expect("persist");
        });
        let id = format!("exchange-{ACCOUNT}-g1");
        with_conn(&db, |conn| {
            let (name, source, group_type): (String, String, String) = conn
                .query_row(
                    "SELECT name, source, group_type FROM contact_groups WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("group row");
            assert_eq!(name, "Group g1");
            assert_eq!(source, DIRECTORY_GROUP_SOURCE);
            assert_eq!(group_type, "distribution_list");
            assert_eq!(
                members_of(conn, &id),
                vec!["fresh@example.test".to_string()]
            );
        });
    }

    #[test]
    fn persist_group_snapshot_prunes_stale() {
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            seed_group(conn, ACCOUNT, "g1", &["a@example.test"]);
            seed_group(conn, ACCOUNT, "g2", &["b@example.test"]);
        });
        in_txn(&db, |tx| {
            persist_group_snapshot(tx, ACCOUNT, &[pulled("g1", Some(&["a@example.test"]))])
                .expect("persist");
        });
        with_conn(&db, |conn| {
            assert_eq!(group_server_ids(conn, ACCOUNT), vec!["g1".to_string()]);
            // Members cascade with the pruned group row.
            assert!(members_of(conn, &format!("exchange-{ACCOUNT}-g2")).is_empty());
        });
    }

    #[test]
    fn persist_group_snapshot_clean_empty_deletes_all() {
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            seed_account(conn, "acct-2");
            seed_group(conn, ACCOUNT, "g1", &["a@example.test"]);
            seed_group(conn, "acct-2", "g9", &["z@example.test"]);
            seed_user_group(conn, "user-1");
        });
        in_txn(&db, |tx| {
            persist_group_snapshot(tx, ACCOUNT, &[]).expect("persist");
        });
        with_conn(&db, |conn| {
            assert!(group_server_ids(conn, ACCOUNT).is_empty());
            assert_eq!(group_server_ids(conn, "acct-2"), vec!["g9".to_string()]);
            let user_groups: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM contact_groups WHERE source = 'user'",
                    [],
                    |row| row.get(0),
                )
                .expect("count user groups");
            assert_eq!(user_groups, 1);
        });
    }

    #[test]
    fn persist_group_snapshot_expansion_failure_keeps_members() {
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            seed_group(conn, ACCOUNT, "g1", &["keep@example.test"]);
        });
        in_txn(&db, |tx| {
            persist_group_snapshot(tx, ACCOUNT, &[pulled("g1", None)]).expect("persist");
        });
        let id = format!("exchange-{ACCOUNT}-g1");
        with_conn(&db, |conn| {
            // The group row is still upserted - and so protected from the
            // prune - while its member rows survive the failed expansion.
            assert_eq!(group_server_ids(conn, ACCOUNT), vec!["g1".to_string()]);
            assert_eq!(group_name_of(conn, &id), "Group g1");
            assert_eq!(members_of(conn, &id), vec!["keep@example.test".to_string()]);
        });
    }

    #[tokio::test]
    async fn group_pull_unsupported_first_call_is_clean_noop() {
        let collected =
            collect_group_pages(|_| async { Err::<Page<DirectoryGroup>, _>(unsupported_error()) })
                .await
                .expect("first-call Unsupported is not an error");
        assert!(collected.is_none(), "first-call Unsupported is a no-op");

        // The inverse: an Unsupported arriving mid-enumeration is a protocol
        // contradiction and takes the Err path, not the no-op path. The guard
        // is "first call", not "nothing collected yet" - a legal empty first
        // page followed by a failure must still abort.
        let mut call = 0;
        let mid = collect_group_pages(|_| {
            call += 1;
            let first = call == 1;
            async move {
                if first {
                    Ok(page(Vec::new(), Some("page-2")))
                } else {
                    Err(unsupported_error())
                }
            }
        })
        .await;
        assert!(mid.is_err(), "mid-enumeration Unsupported must abort");
    }

    #[tokio::test]
    async fn group_pull_partial_enumeration_writes_nothing() {
        // Page 1 succeeds carrying only the first group; page 2 errors with a
        // NoPermission. The enumeration must abort, so the caller never
        // persists - and therefore never prunes against - a partial snapshot.
        let mut call = 0;
        let result = collect_group_pages(|_| {
            call += 1;
            let first = call == 1;
            async move {
                if first {
                    Ok(page(
                        vec![group("g1", DirectoryGroupKind::DistributionList)],
                        Some("page-2"),
                    ))
                } else {
                    Err(no_permission_error())
                }
            }
        })
        .await;
        assert!(result.is_err(), "a failed page aborts the whole pull");

        // Persist is never reached, so both seeded groups and every member
        // row survive untouched.
        let (db, _tmp) = fresh_db();
        with_conn(&db, |conn| {
            seed_account(conn, ACCOUNT);
            seed_group(conn, ACCOUNT, "g1", &["a@example.test"]);
            seed_group(conn, ACCOUNT, "g2", &["b@example.test"]);
        });
        with_conn(&db, |conn| {
            assert_eq!(
                group_server_ids(conn, ACCOUNT),
                vec!["g1".to_string(), "g2".to_string()]
            );
            assert_eq!(
                members_of(conn, &format!("exchange-{ACCOUNT}-g2")),
                vec!["b@example.test".to_string()]
            );
        });
    }

    #[tokio::test]
    async fn group_pull_pages_to_exhaustion() {
        let mut call = 0;
        let collected = collect_group_pages(|cursor| {
            call += 1;
            let first = call == 1;
            async move {
                if first {
                    assert!(cursor.is_none(), "first call carries no cursor");
                    Ok(page(
                        vec![group("g1", DirectoryGroupKind::Unified)],
                        Some("page-2"),
                    ))
                } else {
                    assert_eq!(cursor.as_deref(), Some(b"page-2".as_slice()));
                    Ok(page(
                        vec![group("g2", DirectoryGroupKind::MailEnabledSecurity)],
                        None,
                    ))
                }
            }
        })
        .await
        .expect("paged enumeration")
        .expect("supported");
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].id.0, "g1");
        assert_eq!(collected[1].id.0, "g2");
    }
}
