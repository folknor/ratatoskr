use crate::db::DbState;
/// Look up the provider type for an account from the database.
pub async fn get_provider_type(db: &DbState, account_id: &str) -> Result<String, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT provider FROM accounts WHERE id = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_row([&aid], |row| row.get::<_, String>(0))
            .map_err(|e| format!("No account found for {aid}: {e}"))
    })
    .await
}
