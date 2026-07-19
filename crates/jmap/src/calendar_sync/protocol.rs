use bifrost_jmap::Get;
use bifrost_jmap::calendar_event::{CalendarEvent, CalendarEventGet};

use crate::client::JmapClient;

// ── Internal helpers ───────────────────────────────────────

/// Fetch a batch of calendar events by ID.
pub(super) async fn fetch_event_batch(
    client: &JmapClient,
    ids: &[&str],
) -> Result<Vec<CalendarEvent<Get>>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut get = CalendarEventGet::new(&req_account_id);
    get.ids(ids.iter().copied());
    let handle = request
        .call(get)
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))?;

    response
        .get(&handle)
        .map(|mut r| r.take_list())
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))
}
