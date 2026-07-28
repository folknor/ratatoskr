use db::db::WriterPool;

/// The `settings` key holding a provider's auxiliary cadence counter.
/// Production increments it; the harness affordance seeds it. Both must
/// derive the key here so they cannot drift apart.
fn provider_sync_cycle_key(provider_key: &str, account_id: &str) -> String {
    format!("{provider_key}_sync_cycle:{account_id}")
}

async fn increment_provider_sync_cycle(
    db: &WriterPool,
    account_id: &str,
    provider_key: &'static str,
    provider_label: &'static str,
    overflow_cycle: u32,
) -> Result<u32, String> {
    let key = provider_sync_cycle_key(provider_key, account_id);

    db.with_write(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin {provider_key} sync cycle tx: {e}"))?;
        let stored = db::db::queries::get_setting(&tx, &key)?;
        let current = match stored.as_deref() {
            Some(value) => match value.parse::<u32>() {
                Ok(parsed) => parsed,
                Err(e) => {
                    log::warn!(
                        "Invalid {provider_label} delta cycle value {value:?} for {key}: {e}"
                    );
                    0
                }
            },
            None => 0,
        };
        // Counts delta syncs only. Initial sync runs its own bootstrap work.
        // Overflow lands on the provider's contact-cadence boundary so a wrap
        // still runs the rare contact tier instead of skipping it.
        let next = current.checked_add(1).unwrap_or(overflow_cycle);
        db::db::queries::set_setting(&tx, &key, &next.to_string())?;
        tx.commit()
            .map_err(|e| format!("commit {provider_key} sync cycle tx: {e}"))?;
        Ok(next)
    })
    .await
}

pub async fn increment_graph_sync_cycle(db: &WriterPool, account_id: &str) -> Result<u32, String> {
    increment_provider_sync_cycle(db, account_id, "graph", "Graph", 20).await
}

/// Test-only companion to the production Graph auxiliary cadence counter.
/// Keeping this write here makes harnesses exercise the same settings key and
/// persistence route as `increment_graph_sync_cycle` rather than reaching
/// into the settings table themselves.
pub async fn set_graph_sync_cycle(
    db: &WriterPool,
    account_id: &str,
    cycle: u32,
) -> Result<(), String> {
    let key = provider_sync_cycle_key("graph", account_id);
    db.with_write(move |conn| db::db::queries::set_setting(conn, &key, &cycle.to_string()))
        .await
}

// The `shared_mailboxes.email_address` cache lost its async helpers in B15d.
// The write now lives inside `bifrost::containers::reconcile_namespace_registry`,
// where it shares the transaction that INSERTs the row it updates; the read is
// `rtsk::db::queries_extra::get_shared_mailbox_email_sync`, called from the
// pop-out compose window.

#[cfg(test)]
mod tests {
    use super::provider_sync_cycle_key;

    /// The harness seeder and the production incrementer must address the
    /// same `settings` row, otherwise a cadence-crossing gate silently
    /// drives cycle 1 instead of the value it asked for.
    #[test]
    fn graph_cycle_key_is_shared_between_seed_and_increment() {
        assert_eq!(
            provider_sync_cycle_key("graph", "account-1"),
            "graph_sync_cycle:account-1"
        );
    }
}
