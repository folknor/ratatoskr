use std::collections::HashSet;

use bifrost_sync::{Error, SyncEngine};
use bifrost_types::{AccountId, AddressBookId, ContactCard, ContactCorpus, ProtocolKind};
use db::db::WriteTxn;
use db::db::queries_extra::{
    delete_seen_address_google_other, reconcile_contact_claims_sync, upsert_contact_claim_sync,
    upsert_contact_sync, upsert_seen_address_google_other,
};
use service_state::WriteDbState;

use super::map::{ContactProjection, contact_write_rows};
use crate::bifrost::BifrostProviderKind;

#[derive(Debug, Clone, Copy)]
pub struct ContactPullPolicy {
    pub source: &'static str,
    pub projection: ContactProjection,
}

pub fn policy_for_protocol(protocol: ProtocolKind) -> Option<ContactPullPolicy> {
    match protocol {
        ProtocolKind::Jmap => Some(ContactPullPolicy {
            source: "jmap",
            projection: ContactProjection::PackSecondEmail,
        }),
        ProtocolKind::Graph => Some(ContactPullPolicy {
            source: "graph",
            projection: ContactProjection::FanOutPerEmail,
        }),
        ProtocolKind::Gmail => Some(ContactPullPolicy {
            source: "google",
            projection: ContactProjection::PackSecondEmail,
        }),
        ProtocolKind::CardDav => Some(ContactPullPolicy {
            source: "carddav",
            projection: ContactProjection::PackSecondEmail,
        }),
        _ => None,
    }
}

#[must_use]
pub fn should_pull_on_cycle(provider: BifrostProviderKind, cycle: u32) -> bool {
    let divisor = match provider {
        BifrostProviderKind::Jmap => 1,
        BifrostProviderKind::Graph | BifrostProviderKind::Imap => 5,
        BifrostProviderKind::Gmail => 20,
    };
    cycle.is_multiple_of(divisor)
}

/// Pull the full account contact snapshot and reconcile remote claims atomically.
pub async fn run_contact_pull(
    engine: &SyncEngine,
    account_id: &str,
    provider: BifrostProviderKind,
    write_db: &WriteDbState,
) -> Result<usize, String> {
    let account = AccountId(account_id.to_string());
    let address_books = match engine.address_books_list(&account).await {
        Ok(address_books) => address_books,
        Err(error) if is_unsupported(&error) => return Ok(0),
        Err(error) => return Err(format!("contact address books: {error}")),
    };
    // Native providers use the account-wide route. CardDAV multiget hydration
    // is address-book scoped, so its contact provenance selects the per-book
    // path before any pull. The unsupported fallback keeps the contract sound
    // for future scoped-only providers without fanning Google out over groups.
    let carddav_books = address_books
        .iter()
        .filter(|book| book.provenance.provider == ProtocolKind::CardDav)
        .collect::<Vec<_>>();
    let (mut cards, mut failed_ids) = if !carddav_books.is_empty() {
        let mut cards = Vec::new();
        let mut failed_ids = HashSet::new();
        for address_book in carddav_books {
            let (book_cards, book_failed_ids) =
                collect_contact_pages(engine, &account, Some(address_book.id.clone()))
                    .await
                    .map_err(|error| format!("CardDAV address-book pull: {error}"))?;
            cards.extend(book_cards);
            failed_ids.extend(book_failed_ids);
        }
        (cards, failed_ids)
    } else {
        match collect_contact_pages(engine, &account, None).await {
            Ok(snapshot) => snapshot,
            Err(error) if is_unsupported(&error) => {
                let mut cards = Vec::new();
                let mut failed_ids = HashSet::new();
                for address_book in &address_books {
                    let (book_cards, book_failed_ids) =
                        collect_contact_pages(engine, &account, Some(address_book.id.clone()))
                            .await
                            .map_err(|error| format!("contact address-book pull: {error}"))?;
                    cards.extend(book_cards);
                    failed_ids.extend(book_failed_ids);
                }
                (cards, failed_ids)
            }
            Err(error) => return Err(format!("contact pull: {error}")),
        }
    };
    // Google exposes its auto-collected People corpus as a distinct synthetic
    // address book. It is deliberately the sole per-book pull after the
    // account-wide snapshot: regular Google groups would each re-download the
    // main corpus, while this book is a separate endpoint and must land in
    // `seen_addresses` rather than `contacts`.
    for address_book in address_books
        .iter()
        .filter(|book| book.corpus == ContactCorpus::OtherAutoCollected)
    {
        let (other_cards, other_failed_ids) =
            collect_contact_pages(engine, &account, Some(address_book.id.clone()))
                .await
                .map_err(|error| format!("other contacts pull: {error}"))?;
        cards.extend(other_cards);
        failed_ids.extend(other_failed_ids);
    }
    let account_id = account_id.to_string();
    let source_candidates: Vec<&'static str> = match provider {
        BifrostProviderKind::Jmap => vec!["jmap"],
        BifrostProviderKind::Graph => vec!["graph"],
        BifrostProviderKind::Gmail => vec!["google", "google_other"],
        BifrostProviderKind::Imap => vec!["carddav"],
    };
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("begin contact pull: {error}"))?;
            let mut fetched_by_source: std::collections::HashMap<&str, HashSet<String>> =
                std::collections::HashMap::new();
            let mut count = 0;
            for card in cards {
                let Some(policy) = policy_for_protocol(card.provenance.provider) else {
                    continue;
                };
                if card.corpus == ContactCorpus::OtherAutoCollected {
                    for email in &card.emails {
                        let email = email.value.trim().to_lowercase();
                        if email.is_empty() {
                            continue;
                        }
                        upsert_seen_address_google_other(
                            &tx,
                            &email,
                            &account_id,
                            card.display_name.as_deref(),
                            chrono::Utc::now().timestamp(),
                        )?;
                        upsert_contact_claim_sync(
                            &tx,
                            &account_id,
                            "google_other",
                            &card.native_id,
                            &email,
                            card.address_book_id.as_ref().map(|id| id.0.as_str()),
                            "other",
                        )?;
                        fetched_by_source
                            .entry("google_other")
                            .or_default()
                            .insert(card.native_id.clone());
                        count += 1;
                    }
                    continue;
                }
                let rows = contact_write_rows(&card, &account_id, policy.source, policy.projection);
                for row in rows {
                    upsert_contact_sync(&tx, &row)?;
                    upsert_contact_claim_sync(
                        &tx,
                        &account_id,
                        policy.source,
                        &card.native_id,
                        &row.email,
                        card.address_book_id.as_ref().map(|id| id.0.as_str()),
                        "main",
                    )?;
                    count += 1;
                }
                fetched_by_source
                    .entry(policy.source)
                    .or_default()
                    .insert(card.native_id);
            }
            for source in source_candidates {
                let fetched = fetched_by_source.remove(source).unwrap_or_default();
                reconcile_source(&tx, &account_id, source, &fetched, &failed_ids)?;
            }
            tx.commit()
                .map_err(|error| format!("commit contact pull: {error}"))?;
            Ok(count)
        })
        .await
}

fn is_unsupported(error: &Error) -> bool {
    matches!(
        error,
        Error::Account(account_error)
            if matches!(account_error.recovery(), bifrost_types::RecoveryClass::Unsupported(_))
    )
}

async fn collect_contact_pages(
    engine: &SyncEngine,
    account: &AccountId,
    address_book: Option<AddressBookId>,
) -> Result<(Vec<ContactCard>, HashSet<String>), Error> {
    let mut cursor = None;
    let mut cards = Vec::new();
    let mut failed_ids = HashSet::new();
    loop {
        let page = engine
            .contacts_list(account, address_book.clone(), cursor)
            .await?;
        failed_ids.extend(page.failed_ids);
        cards.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((cards, failed_ids));
        }
    }
}

/// Retire the claims a completed snapshot for `source` no longer contains and
/// prune the now-unclaimed local materialization (§ 5.3 snapshot reconcile).
///
/// The guard rules, all load-bearing:
/// - `failed_ids` are transient per-resource hydration failures, never treated
///   as deletions (they are excluded from the "absent -> retire" set inside
///   `reconcile_contact_claims_sync`), so a page that failed to hydrate the
///   whole prior snapshot prunes nothing, and a clean empty enumeration
///   (empty fetched, empty `failed_ids`) legitimately deletes-all.
/// - The retire is source-scoped: a `"google"` pass only touches `"google"`
///   claims, never `"google_other"` (or another provider's) claims.
/// - A `contacts` row is deleted only when NO remaining claim (any source /
///   account) references its email, preserving cross-provider dedup; a locally
///   authored `source='user'` row is never pruned even when its email was also
///   claimed by a provider that has now dropped it.
/// - The `google_other` corpus prunes `seen_addresses`, not `contacts`.
fn reconcile_source(
    tx: &WriteTxn<'_>,
    account_id: &str,
    source: &str,
    fetched: &HashSet<String>,
    failed_ids: &HashSet<String>,
) -> Result<(), String> {
    let orphaned = reconcile_contact_claims_sync(tx, account_id, source, fetched, failed_ids)?;
    for email in orphaned {
        if source == "google_other" {
            delete_seen_address_google_other(tx, &email, account_id)?;
        } else {
            // Never prune a locally authored contact: the upsert preserves
            // `source='user'` against provider writes, so the reconcile must
            // not delete one just because the provider that also claimed its
            // email dropped it.
            tx.execute(
                "DELETE FROM contacts WHERE email = ?1 AND source != 'user'",
                rusqlite::params![email],
            )
            .map_err(|e| format!("delete orphan contact: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::db::WriteConn;
    use rusqlite::params;
    use service_state::WriteDbState;
    use tempfile::TempDir;

    const ACCOUNT: &str = "acct-1";

    fn fresh_db() -> (WriteDbState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let pool = db::db::open_writer_pool(tmp.path()).expect("writer pool");
        (WriteDbState::from_pool(pool), tmp)
    }

    fn seed_account(conn: &WriteConn<'_>, account_id: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, email) VALUES (?1, ?2)",
            params![account_id, format!("{account_id}@example.test")],
        )
        .expect("seed account");
    }

    fn seed_contact(conn: &WriteConn<'_>, email: &str, source: &str) {
        conn.execute(
            "INSERT INTO contacts (id, email, source, account_id, server_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                format!("{source}-{ACCOUNT}-{email}"),
                email,
                source,
                ACCOUNT,
                format!("srv-{email}")
            ],
        )
        .expect("seed contact");
    }

    fn seed_seen_google_other(conn: &WriteConn<'_>, email: &str) {
        conn.execute(
            "INSERT INTO seen_addresses \
             (email, account_id, source, first_seen_at, last_seen_at) \
             VALUES (?1, ?2, 'google_other', 0, 0)",
            params![email, ACCOUNT],
        )
        .expect("seed seen address");
    }

    fn seed_claim(conn: &WriteConn<'_>, source: &str, server_id: &str, email: &str) {
        upsert_contact_claim_sync(conn, ACCOUNT, source, server_id, email, None, "main")
            .expect("seed claim");
    }

    fn contact_exists(conn: &WriteConn<'_>, email: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM contacts WHERE email = ?1",
            params![email],
            |row| row.get::<_, i64>(0),
        )
        .expect("count contacts")
            > 0
    }

    fn claim_exists(conn: &WriteConn<'_>, source: &str, server_id: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM contact_claims WHERE source = ?1 AND server_id = ?2",
            params![source, server_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("count claims")
            > 0
    }

    fn seen_exists(conn: &WriteConn<'_>, email: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM seen_addresses WHERE email = ?1",
            params![email],
            |row| row.get::<_, i64>(0),
        )
        .expect("count seen")
            > 0
    }

    fn empty() -> HashSet<String> {
        HashSet::new()
    }

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    /// A `server_id` the provider tried but failed to hydrate is transient, not
    /// a remote delete: its claim survives and the materialized row stays, even
    /// when the fetched set is empty (the whole snapshot failed to hydrate).
    #[test]
    fn contact_pull_reconcile_excludes_failed_ids() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write_sync(|conn| {
                seed_account(conn, ACCOUNT);
                seed_contact(conn, "a@x.test", "google");
                seed_claim(conn, "google", "srv-A", "a@x.test");

                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "google", &empty(), &set(&["srv-A"]))?;
                tx.commit().map_err(|e| e.to_string())?;

                assert!(claim_exists(conn, "google", "srv-A"), "failed id retained");
                assert!(contact_exists(conn, "a@x.test"), "failed id not pruned");
                Ok(())
            })
            .expect("reconcile");
    }

    /// A clean empty enumeration (empty fetched AND empty `failed_ids`) is a
    /// real delete-all: the claim is retired and the now-unclaimed row deleted.
    #[test]
    fn contact_pull_reconcile_clean_empty_deletes_all() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write_sync(|conn| {
                seed_account(conn, ACCOUNT);
                seed_contact(conn, "a@x.test", "google");
                seed_claim(conn, "google", "srv-A", "a@x.test");

                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "google", &empty(), &empty())?;
                tx.commit().map_err(|e| e.to_string())?;

                assert!(!claim_exists(conn, "google", "srv-A"), "claim retired");
                assert!(!contact_exists(conn, "a@x.test"), "row deleted");
                Ok(())
            })
            .expect("reconcile");
    }

    /// A `"google"` prune is source-scoped: it never retires a `"google_other"`
    /// claim nor prunes its `seen_addresses` corpus.
    #[test]
    fn contact_pull_reconcile_is_source_scoped() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write_sync(|conn| {
                seed_account(conn, ACCOUNT);
                seed_contact(conn, "g@x.test", "google");
                seed_claim(conn, "google", "srv-G", "g@x.test");
                seed_seen_google_other(conn, "o@x.test");
                seed_claim(conn, "google_other", "srv-GO", "o@x.test");

                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "google", &empty(), &empty())?;
                tx.commit().map_err(|e| e.to_string())?;

                assert!(!contact_exists(conn, "g@x.test"), "google row pruned");
                assert!(
                    claim_exists(conn, "google_other", "srv-GO"),
                    "google_other claim untouched by a google prune"
                );
                assert!(
                    seen_exists(conn, "o@x.test"),
                    "google_other seen row untouched"
                );
                Ok(())
            })
            .expect("reconcile");
    }

    /// A locally authored `source='user'` row is never pruned, even when a
    /// provider that also claimed its email drops it.
    #[test]
    fn contact_pull_reconcile_preserves_user_authored_row() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write_sync(|conn| {
                seed_account(conn, ACCOUNT);
                seed_contact(conn, "shared@x.test", "user");
                seed_claim(conn, "google", "srv-S", "shared@x.test");

                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "google", &empty(), &empty())?;
                tx.commit().map_err(|e| e.to_string())?;

                assert!(!claim_exists(conn, "google", "srv-S"), "claim retired");
                assert!(
                    contact_exists(conn, "shared@x.test"),
                    "user-authored row preserved"
                );
                Ok(())
            })
            .expect("reconcile");
    }

    /// Cross-provider dedup guard: one `contacts` row claimed by two providers
    /// survives while ANY claim remains, and is deleted only when the last
    /// claim is retired.
    #[test]
    fn contact_pull_reconcile_keeps_cross_provider_claimed_row() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write_sync(|conn| {
                seed_account(conn, ACCOUNT);
                seed_contact(conn, "dup@x.test", "google");
                seed_claim(conn, "google", "srv-DG", "dup@x.test");
                seed_claim(conn, "graph", "srv-DR", "dup@x.test");

                // Google drops the contact; Graph still claims the email.
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "google", &empty(), &empty())?;
                tx.commit().map_err(|e| e.to_string())?;
                assert!(
                    !claim_exists(conn, "google", "srv-DG"),
                    "google claim retired"
                );
                assert!(claim_exists(conn, "graph", "srv-DR"), "graph claim remains");
                assert!(
                    contact_exists(conn, "dup@x.test"),
                    "row kept while Graph still claims it"
                );

                // Graph drops it too; now no claim references the email.
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                reconcile_source(&tx, ACCOUNT, "graph", &empty(), &empty())?;
                tx.commit().map_err(|e| e.to_string())?;
                assert!(
                    !claim_exists(conn, "graph", "srv-DR"),
                    "graph claim retired"
                );
                assert!(
                    !contact_exists(conn, "dup@x.test"),
                    "row deleted once the last claim is gone"
                );
                Ok(())
            })
            .expect("reconcile");
    }

    #[test]
    fn contact_pull_cadence_preserves_each_source_budget() {
        assert!(should_pull_on_cycle(BifrostProviderKind::Jmap, 0));
        assert!(should_pull_on_cycle(BifrostProviderKind::Jmap, 1));
        assert!(should_pull_on_cycle(BifrostProviderKind::Graph, 0));
        assert!(!should_pull_on_cycle(BifrostProviderKind::Graph, 4));
        assert!(should_pull_on_cycle(BifrostProviderKind::Graph, 5));
        assert!(should_pull_on_cycle(BifrostProviderKind::Imap, 5));
        assert!(should_pull_on_cycle(BifrostProviderKind::Gmail, 0));
        assert!(!should_pull_on_cycle(BifrostProviderKind::Gmail, 19));
        assert!(should_pull_on_cycle(BifrostProviderKind::Gmail, 20));
    }
}
