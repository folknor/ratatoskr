#![cfg(test)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use db::db::params;
use rusqlite::Connection;

// ── Test helpers ────────────────────────────────────────────────────

/// Create a minimal ActionContext backed by in-memory DBs + temp dirs.
/// Returns (ctx, _tmpdir) - keep _tmpdir alive for the test duration.
fn make_test_ctx() -> (ActionContext, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Main DB with full migrations
    let write_db = service_state::WriteDbState::from_pool(
        db::db::open_writer_pool(tmp.path()).expect("open writer pool"),
    );
    let db = db::db::open_reader_pool(tmp.path()).expect("open reader pool");

    // Stores: tempdir-backed
    let body_store = store::body_store::BodyStoreReadState::init(tmp.path()).expect("body store");
    let inline_images =
        store::inline_image_store::InlineImageStoreReadState::init(tmp.path()).expect("inline images");
    let search = search::SearchReadState::init(tmp.path()).expect("search");

    let ctx = ActionContext {
        db,
        write_db,
        body_store,
        inline_images,
        search,
        encryption_key: [0u8; 32],
        suppress_pending_enqueue: false,
        in_flight: Arc::new(Mutex::new(HashSet::new())),
    };
    (ctx, tmp)
}

fn with_test_conn<T>(ctx: &ActionContext, f: impl FnOnce(&Connection) -> T) -> T {
    ctx.write_db
        .with_conn_sync(|conn| Ok(f(conn)))
        .expect("test db access")
}

/// Insert a test account row (needed for FK constraints on pending_operations).
fn insert_test_account(ctx: &ActionContext, account_id: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, provider, is_active) VALUES (?1, ?2, ?3, 1)",
            params![account_id, format!("{account_id}@test.com"), "gmail_api"],
        )
        .expect("insert account");
        for (folder_id, name) in [("INBOX", "Inbox"), ("TRASH", "Trash"), ("SPAM", "Spam")] {
            conn.execute(
                "INSERT OR IGNORE INTO folders (account_id, id, name, is_undeletable)
                 VALUES (?1, ?2, ?3, 1)",
                params![account_id, folder_id, name],
            )
            .expect("insert folder");
        }
    });
}

/// Insert a test thread row.
fn insert_test_thread(ctx: &ActionContext, account_id: &str, thread_id: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO threads (id, account_id, subject, snippet, last_message_at, is_read, is_starred) \
             VALUES (?1, ?2, 'test', '', 0, 0, 0)",
            params![thread_id, account_id],
        )
        .expect("insert thread");
    });
}

/// Insert a thread_folders row.
fn insert_thread_folder(ctx: &ActionContext, account_id: &str, thread_id: &str, folder_id: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) VALUES (?1, ?2, ?3)",
            params![account_id, thread_id, folder_id],
        )
        .expect("insert thread_folder");
    });
}

/// Check if a thread_folders row exists.
fn has_thread_folder(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
) -> bool {
    with_test_conn(ctx, |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2 AND folder_id = ?3",
                params![account_id, thread_id, folder_id],
                |row| row.get(0),
            )
            .expect("query");
        count > 0
    })
}

/// Get thread.is_starred value.
fn get_thread_starred(ctx: &ActionContext, account_id: &str, thread_id: &str) -> bool {
    with_test_conn(ctx, |conn| {
        conn.query_row(
            "SELECT is_starred FROM threads WHERE account_id = ?1 AND id = ?2",
            params![account_id, thread_id],
            |row| row.get::<_, bool>(0),
        )
        .expect("query")
    })
}

/// Count pending ops for a resource.
fn count_pending_ops(
    ctx: &ActionContext,
    account_id: &str,
    resource_id: &str,
    op_type: &str,
) -> i64 {
    with_test_conn(ctx, |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM pending_operations \
             WHERE account_id = ?1 AND resource_id = ?2 AND operation_type = ?3 \
               AND status IN ('pending', 'executing')",
            params![account_id, resource_id, op_type],
            |row| row.get(0),
        )
        .expect("query")
    })
}

// ── FlightGuard tests ───────────────────────────────────────────────

#[test]
fn flight_guard_acquire_and_release() {
    let (ctx, _tmp) = make_test_ctx();

    // Acquire succeeds
    let guard = ctx.try_acquire_flight("acc1", "thread1");
    assert!(guard.is_some(), "first acquire should succeed");
    assert!(ctx.is_in_flight("acc1", "thread1"));

    // Second acquire for same thread fails
    let guard2 = ctx.try_acquire_flight("acc1", "thread1");
    assert!(guard2.is_none(), "duplicate acquire should fail");

    // Different thread succeeds
    let guard3 = ctx.try_acquire_flight("acc1", "thread2");
    assert!(guard3.is_some());

    // Drop first guard - thread1 released
    drop(guard);
    assert!(!ctx.is_in_flight("acc1", "thread1"));

    // Can re-acquire after drop
    let guard4 = ctx.try_acquire_flight("acc1", "thread1");
    assert!(guard4.is_some());

    drop(guard3);
    drop(guard4);
}

#[test]
fn flight_guard_different_accounts_independent() {
    let (ctx, _tmp) = make_test_ctx();

    let g1 = ctx.try_acquire_flight("acc1", "thread1");
    let g2 = ctx.try_acquire_flight("acc2", "thread1");
    assert!(g1.is_some());
    assert!(
        g2.is_some(),
        "same thread_id on different accounts should be independent"
    );
    drop(g1);
    drop(g2);
}

// ── Local mutation tests ────────────────────────────────────────────

#[tokio::test]
async fn archive_local_removes_inbox_label() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_thread_folder(&ctx, "acc1", "t1", "INBOX");
    assert!(has_thread_folder(&ctx, "acc1", "t1", "INBOX"));

    let result = super::archive::archive_local(&ctx, "acc1", "t1").await;
    assert!(result.is_ok());
    assert!(!has_thread_folder(&ctx, "acc1", "t1", "INBOX"));
}

#[tokio::test]
async fn archive_local_is_idempotent() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    // No INBOX label - archive is a no-op but should not error
    let result = super::archive::archive_local(&ctx, "acc1", "t1").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn star_local_sets_flag() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    assert!(!get_thread_starred(&ctx, "acc1", "t1"));

    let result = super::star::star_local(&ctx, "acc1", "t1", true).await;
    assert!(result.is_ok());
    assert!(get_thread_starred(&ctx, "acc1", "t1"));

    // Unstar
    let result = super::star::star_local(&ctx, "acc1", "t1", false).await;
    assert!(result.is_ok());
    assert!(!get_thread_starred(&ctx, "acc1", "t1"));
}

#[tokio::test]
async fn trash_local_removes_inbox_adds_trash() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_thread_folder(&ctx, "acc1", "t1", "INBOX");

    let result = super::trash::trash_local(&ctx, "acc1", "t1").await;
    assert!(result.is_ok());
    assert!(!has_thread_folder(&ctx, "acc1", "t1", "INBOX"));
    assert!(has_thread_folder(&ctx, "acc1", "t1", "TRASH"));
}

// ── Public action tests (no provider → LocalOnly) ───────────────────

#[tokio::test]
async fn archive_nonexistent_thread_does_not_succeed() {
    let (ctx, _tmp) = make_test_ctx();
    // No account/thread in DB → archive_local finds no INBOX label → NoOp or Failed
    let outcome = super::archive::archive(&ctx, "nonexistent", "t1").await;
    assert!(
        !outcome.is_success(),
        "archiving nonexistent thread should not return Success, got {outcome:?}"
    );
}

#[tokio::test]
async fn pin_is_local_only_success() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");

    let outcome = super::pin::pin(&ctx, "acc1", "t1", true).await;
    assert!(outcome.is_success());
}

#[tokio::test]
async fn snooze_sets_state_and_removes_inbox() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_thread_folder(&ctx, "acc1", "t1", "INBOX");

    let outcome = super::snooze::snooze(&ctx, "acc1", "t1", 1234567890).await;
    assert!(outcome.is_success());
    assert!(!has_thread_folder(&ctx, "acc1", "t1", "INBOX"));

    let (is_snoozed, snooze_until): (bool, i64) = with_test_conn(&ctx, |conn| {
        conn.query_row(
            "SELECT is_snoozed, snooze_until FROM threads WHERE account_id = 'acc1' AND id = 't1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query")
    });
    assert!(is_snoozed);
    assert_eq!(snooze_until, 1234567890);
}

#[tokio::test]
async fn unsnooze_restores_inbox() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");

    // Snooze first
    super::snooze::snooze(&ctx, "acc1", "t1", 999).await;

    // Unsnooze
    let outcome = super::snooze::unsnooze(&ctx, "acc1", "t1").await;
    assert!(outcome.is_success());
    assert!(has_thread_folder(&ctx, "acc1", "t1", "INBOX"));
}

// ── Pending-ops dedup tests ─────────────────────────────────────────

#[tokio::test]
async fn enqueue_replaces_existing_pending_op() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");

    // First enqueue
    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("network error"),
        retryable: true,
    };
    super::pending::enqueue_if_retryable(
        &ctx,
        &outcome,
        "acc1",
        "star",
        "t1",
        r#"{"starred":true}"#,
    )
    .await;
    assert_eq!(count_pending_ops(&ctx, "acc1", "t1", "star"), 1);

    // Second enqueue with opposite params - should replace, not duplicate
    super::pending::enqueue_if_retryable(
        &ctx,
        &outcome,
        "acc1",
        "star",
        "t1",
        r#"{"starred":false}"#,
    )
    .await;
    assert_eq!(count_pending_ops(&ctx, "acc1", "t1", "star"), 1);

    // Verify params were updated to the latest
    let params: String = with_test_conn(&ctx, |conn| {
        conn.query_row(
            "SELECT params FROM pending_operations WHERE account_id = 'acc1' AND resource_id = 't1' AND operation_type = 'star'",
            [],
            |row| row.get(0),
        )
        .expect("query")
    });
    assert_eq!(params, r#"{"starred":false}"#);
}

#[tokio::test]
async fn enqueue_suppressed_when_flag_set() {
    let (mut ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    ctx.suppress_pending_enqueue = true;

    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("network error"),
        retryable: true,
    };
    super::pending::enqueue_if_retryable(&ctx, &outcome, "acc1", "star", "t1", "{}").await;
    assert_eq!(count_pending_ops(&ctx, "acc1", "t1", "star"), 0);
}

#[tokio::test]
async fn enqueue_skipped_for_permanent_errors() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");

    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote_with_kind(RemoteFailureKind::Permanent, "forbidden"),
        retryable: true, // policy says retry, but error kind overrides
    };
    super::pending::enqueue_if_retryable(&ctx, &outcome, "acc1", "star", "t1", "{}").await;
    assert_eq!(count_pending_ops(&ctx, "acc1", "t1", "star"), 0);
}

// ── Per-type retry policy tests ─────────────────────────────────────

#[tokio::test]
async fn folder_actions_get_10_max_retries() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");

    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("error"),
        retryable: true,
    };
    super::pending::enqueue_if_retryable(&ctx, &outcome, "acc1", "archive", "t1", "{}").await;

    let max: i64 = with_test_conn(&ctx, |conn| {
        conn.query_row(
            "SELECT max_retries FROM pending_operations WHERE resource_id = 't1'",
            [],
            |row| row.get(0),
        )
        .expect("query")
    });
    assert_eq!(max, 10);
}

#[tokio::test]
async fn flag_actions_get_5_max_retries() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");

    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("error"),
        retryable: true,
    };
    super::pending::enqueue_if_retryable(&ctx, &outcome, "acc1", "star", "t1", "{}").await;

    let max: i64 = with_test_conn(&ctx, |conn| {
        conn.query_row(
            "SELECT max_retries FROM pending_operations WHERE resource_id = 't1'",
            [],
            |row| row.get(0),
        )
        .expect("query")
    });
    assert_eq!(max, 5);
}

// ── Batch executor tests ────────────────────────────────────────────

#[tokio::test]
async fn batch_pin_is_local_only_success() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_test_thread(&ctx, "acc1", "t2");

    let outcomes = super::batch::batch_execute(
        &ctx,
        vec![
            (
                "acc1".to_string(),
                "t1".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
            (
                "acc1".to_string(),
                "t2".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
        ],
    )
    .await;

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes[0].is_success());
    assert!(outcomes[1].is_success());
}

#[tokio::test]
async fn batch_preserves_target_order() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_account(&ctx, "acc2");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_test_thread(&ctx, "acc2", "t2");
    insert_test_thread(&ctx, "acc1", "t3");

    // Mixed accounts - outcomes should be in same order as targets
    let outcomes = super::batch::batch_execute(
        &ctx,
        vec![
            (
                "acc1".to_string(),
                "t1".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
            (
                "acc2".to_string(),
                "t2".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
            (
                "acc1".to_string(),
                "t3".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
        ],
    )
    .await;

    assert_eq!(outcomes.len(), 3);
    // All should succeed for pin (local-only)
    assert!(outcomes.iter().all(ActionOutcome::is_success));
}

#[tokio::test]
async fn batch_archive_without_accounts_returns_local_only() {
    let (ctx, _tmp) = make_test_ctx();
    // No accounts → create_provider fails → degraded path

    let outcomes = super::batch::batch_execute(
        &ctx,
        vec![
            (
                "nonexistent".to_string(),
                "t1".to_string(),
                super::MailOperation::Archive,
            ),
            (
                "nonexistent".to_string(),
                "t2".to_string(),
                super::MailOperation::Archive,
            ),
        ],
    )
    .await;

    assert_eq!(outcomes.len(), 2);
    // Both should be LocalOnly or Failed (provider creation fails)
    for o in &outcomes {
        assert!(o.is_local_only() || o.is_failed());
    }
}

#[tokio::test]
async fn batch_respects_flight_guard() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_test_thread(&ctx, "acc1", "t2");

    // Hold flight guard on t1
    let _guard = ctx.try_acquire_flight("acc1", "t1");

    let outcomes = super::batch::batch_execute(
        &ctx,
        vec![
            (
                "acc1".to_string(),
                "t1".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
            (
                "acc1".to_string(),
                "t2".to_string(),
                super::MailOperation::SetPinned { to: true },
            ),
        ],
    )
    .await;

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes[0].is_failed(), "t1 should fail - in flight");
    assert!(outcomes[1].is_success(), "t2 should succeed");
}

// ── ActionOutcome / ActionError helper tests ────────────────────────

#[test]
fn action_error_retryable_classification() {
    assert!(ActionError::remote("err").is_retryable()); // Unknown → retryable
    assert!(ActionError::remote_with_kind(RemoteFailureKind::Transient, "err").is_retryable());
    assert!(!ActionError::remote_with_kind(RemoteFailureKind::Permanent, "err").is_retryable());
    assert!(
        !ActionError::remote_with_kind(RemoteFailureKind::NotImplemented, "err").is_retryable()
    );
    assert!(!ActionError::db("err").is_retryable());
    assert!(!ActionError::not_found("err").is_retryable());
}

#[test]
fn action_outcome_helpers() {
    assert!(ActionOutcome::Success.is_success());
    assert!(!ActionOutcome::Success.is_failed());
    assert!(!ActionOutcome::Success.is_local_only());

    let lo = ActionOutcome::LocalOnly {
        reason: ActionError::remote("err"),
        retryable: true,
    };
    assert!(lo.is_local_only());
    assert!(!lo.is_success());

    let failed = ActionOutcome::Failed {
        error: ActionError::db("err"),
    };
    assert!(failed.is_failed());
}

// ── label_group composite tests ────────────────────────────────────

fn insert_label_group(ctx: &ActionContext, group_id: i64, name: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT INTO label_groups (id, name, color_bg, color_fg) VALUES (?1, ?2, '#000', '#fff')",
            params![group_id, name],
        )
        .expect("insert label group");
    });
}

fn insert_label(ctx: &ActionContext, account_id: &str, label_id: &str, name: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT INTO labels (account_id, id, name) VALUES (?1, ?2, ?3)",
            params![account_id, label_id, name],
        )
        .expect("insert label");
    });
}

fn insert_group_member(ctx: &ActionContext, group_id: i64, account_id: &str, label_id: &str) {
    with_test_conn(ctx, |conn| {
        conn.execute(
            "INSERT INTO label_group_members (group_id, account_id, label_id) VALUES (?1, ?2, ?3)",
            params![group_id, account_id, label_id],
        )
        .expect("insert member");
    });
}

fn has_pending_label_intent(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
    op: &str,
) -> bool {
    with_test_conn(ctx, |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_thread_label_intents \
                 WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3 AND op = ?4",
                params![account_id, thread_id, label_id, op],
                |row| row.get(0),
            )
            .expect("query");
        count > 0
    })
}

#[tokio::test]
async fn apply_label_group_local_initial_writes_member_intent() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_label_group(&ctx, 1, "Work");
    insert_label(&ctx, "acc1", "labelA", "Label A");
    insert_group_member(&ctx, 1, "acc1", "labelA");

    let r = super::label_group::apply_label_group_local_initial(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await;
    assert!(r.is_ok(), "{r:?}");
    assert!(has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Add"));
}

#[tokio::test]
async fn apply_label_group_local_initial_zero_members_still_succeeds() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_label_group(&ctx, 1, "Work");
    // No members - per redesign.md a zero-member group is valid and the
    // composite resolves locally.
    let r = super::label_group::apply_label_group_local_initial(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await;
    assert!(r.is_ok());
    assert!(!has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Add"));
}

#[tokio::test]
async fn apply_label_group_local_initial_unknown_group_errors() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    let r = super::label_group::apply_label_group_local_initial(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(99),
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn remove_label_group_local_initial_writes_remove_intent() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_label_group(&ctx, 1, "Work");
    insert_label(&ctx, "acc1", "labelA", "Label A");
    insert_group_member(&ctx, 1, "acc1", "labelA");
    super::label_group::apply_label_group_local_initial(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await
    .expect("apply");
    assert!(has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Add"));

    super::label_group::remove_label_group_local_initial(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await
    .expect("remove");
    assert!(has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Remove"));
}

#[tokio::test]
async fn remove_label_group_with_no_attachment_succeeds() {
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_label_group(&ctx, 1, "Work");
    // Never applied - remove must still resolve cleanly (idempotent).
    let outcome = super::label_group::remove_label_group(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await;
    assert!(outcome.is_local_only() || outcome.is_success());
    assert!(!has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Remove"));
}

#[tokio::test]
async fn apply_label_group_handles_member_label() {
    // Initial dispatch through the public path; no provider available
    // (no oauth setup), so it falls through the LocalOnly enqueue branch.
    // We assert the local mutation lands.
    let (ctx, _tmp) = make_test_ctx();
    insert_test_account(&ctx, "acc1");
    insert_test_thread(&ctx, "acc1", "t1");
    insert_label_group(&ctx, 1, "Work");
    insert_label(&ctx, "acc1", "labelA", "Label A");
    insert_group_member(&ctx, 1, "acc1", "labelA");

    let outcome = super::label_group::apply_label_group(
        &ctx,
        "acc1",
        "t1",
        common::typed_ids::LabelGroupId(1),
    )
    .await;
    // Provider construction will fail for this fake account; expect
    // LocalOnly with the composite-typed retry enqueued (NOT per-member
    // rows - that bypasses the preflight contract).
    assert!(outcome.is_local_only() || outcome.is_success());
    assert!(has_pending_label_intent(&ctx, "acc1", "t1", "labelA", "Add"));
    // Composite type enqueued, not raw addLabel.
    let composite_count = count_pending_ops(&ctx, "acc1", "t1", "applyLabelGroup");
    let raw_count = count_pending_ops(&ctx, "acc1", "t1", "addLabel");
    assert!(
        composite_count > 0 || raw_count == 0,
        "expect composite-typed retry rather than raw addLabel rows: \
         composite={composite_count}, raw={raw_count}"
    );
    assert_eq!(raw_count, 0, "no per-member rows allowed in pending_operations");
}
