use crate::imap::client as imap_client;
use crate::imap::types::{
    DeltaCheckRequest, DeltaCheckResult, ImapConfig, ImapFetchResult, ImapFolder,
    ImapFolderSearchResult, ImapFolderStatus, ImapFolderSyncResult, ImapMessage,
};
use crate::smtp::client as smtp_client;
use crate::smtp::types::{SmtpConfig, SmtpSendResult};

// ---------- IMAP session helper ----------

/// Connect to IMAP, run the body expression with `$session`, then logout.
/// Ensures logout runs regardless of whether the work succeeds or fails.
///
/// Usage:
///   with_imap_session!(&config, session => {
///       imap_client::list_folders(&mut session).await
///   })
macro_rules! with_imap_session {
    ($config:expr, $session:ident => $body:expr) => {{
        let mut $session = imap_client::connect($config).await?;
        let result = $body;
        drop($session.logout().await);
        result
    }};
}

// ---------- IMAP commands ----------

#[tauri::command]
pub async fn imap_test_connection(config: ImapConfig) -> Result<String, String> {
    imap_client::test_connection(&config).await
}

#[tauri::command]
pub async fn imap_list_folders(config: ImapConfig) -> Result<Vec<ImapFolder>, String> {
    with_imap_session!(&config, session => {
        imap_client::list_folders(&mut session).await
    })
}

#[tauri::command]
pub async fn imap_fetch_messages(
    config: ImapConfig,
    folder: String,
    uids: Vec<u32>,
) -> Result<ImapFetchResult, String> {
    if uids.is_empty() {
        return Err("No UIDs provided".to_string());
    }

    // Build a UID set string like "1,5,10,20"
    let uid_set: String = uids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");

    let result = with_imap_session!(&config, session => {
        imap_client::fetch_messages(&mut session, &folder, &uid_set).await
    });

    match result {
        Ok(r) => Ok(r),
        Err(e) if e.starts_with("ASYNC_IMAP_EMPTY:") => {
            // async-imap can't parse this server's responses — use raw TCP fallback
            log::info!("Falling back to raw TCP fetch for folder {folder}");
            imap_client::raw_fetch_messages(&config, &folder, &uid_set).await
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn imap_fetch_new_uids(
    config: ImapConfig,
    folder: String,
    since_uid: u32,
) -> Result<Vec<u32>, String> {
    with_imap_session!(&config, session => {
        imap_client::fetch_new_uids(&mut session, &folder, since_uid).await
    })
}

#[tauri::command]
pub async fn imap_search_all_uids(
    config: ImapConfig,
    folder: String,
) -> Result<Vec<u32>, String> {
    with_imap_session!(&config, session => {
        imap_client::search_all_uids(&mut session, &folder).await
    })
}

#[tauri::command]
pub async fn imap_fetch_message_body(
    config: ImapConfig,
    folder: String,
    uid: u32,
) -> Result<ImapMessage, String> {
    with_imap_session!(&config, session => {
        imap_client::fetch_message_body(&mut session, &folder, uid).await
    })
}

#[tauri::command]
pub async fn imap_fetch_raw_message(
    config: ImapConfig,
    folder: String,
    uid: u32,
) -> Result<String, String> {
    with_imap_session!(&config, session => {
        imap_client::fetch_raw_message(&mut session, &folder, uid).await
    })
}

#[tauri::command]
pub async fn imap_set_flags(
    config: ImapConfig,
    folder: String,
    uids: Vec<u32>,
    flags: Vec<String>,
    add: bool,
) -> Result<(), String> {
    if uids.is_empty() {
        return Ok(());
    }

    let uid_set: String = uids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");

    let flag_op = if add { "+FLAGS" } else { "-FLAGS" };

    // Format flags like "(\Seen \Flagged)"
    let flags_str = format!(
        "({})",
        flags
            .iter()
            .map(|f| {
                // Ensure flags have the backslash prefix if they're standard flags
                if f.starts_with('\\') {
                    f.clone()
                } else {
                    format!("\\{f}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    );

    with_imap_session!(&config, session => {
        imap_client::set_flags(&mut session, &folder, &uid_set, flag_op, &flags_str).await
    })
}

#[tauri::command]
pub async fn imap_move_messages(
    config: ImapConfig,
    folder: String,
    uids: Vec<u32>,
    destination: String,
) -> Result<(), String> {
    if uids.is_empty() {
        return Ok(());
    }

    let uid_set: String = uids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");

    with_imap_session!(&config, session => {
        imap_client::move_messages(&mut session, &folder, &uid_set, &destination).await
    })
}

#[tauri::command]
pub async fn imap_delete_messages(
    config: ImapConfig,
    folder: String,
    uids: Vec<u32>,
) -> Result<(), String> {
    if uids.is_empty() {
        return Ok(());
    }

    let uid_set: String = uids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");

    with_imap_session!(&config, session => {
        imap_client::delete_messages(&mut session, &folder, &uid_set).await
    })
}

#[tauri::command]
pub async fn imap_get_folder_status(
    config: ImapConfig,
    folder: String,
) -> Result<ImapFolderStatus, String> {
    with_imap_session!(&config, session => {
        imap_client::get_folder_status(&mut session, &folder).await
    })
}

#[tauri::command]
pub async fn imap_fetch_attachment(
    config: ImapConfig,
    folder: String,
    uid: u32,
    part_id: String,
) -> Result<String, String> {
    with_imap_session!(&config, session => {
        imap_client::fetch_attachment(&mut session, &folder, uid, &part_id).await
    })
}

#[tauri::command]
pub async fn imap_append_message(
    config: ImapConfig,
    folder: String,
    flags: Option<String>,
    raw_message: String,
) -> Result<(), String> {
    // raw_message is base64url-encoded; decode it before entering the session
    let raw_bytes = base64url_decode(&raw_message)?;

    with_imap_session!(&config, session => {
        let flags_ref = flags.as_deref();
        imap_client::append_message(&mut session, &folder, flags_ref, &raw_bytes).await
    })
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    engine
        .decode(input)
        .map_err(|e| format!("base64url decode failed: {e}"))
}

#[tauri::command]
pub async fn imap_search_folder(
    config: ImapConfig,
    folder: String,
    since_date: Option<String>,
) -> Result<ImapFolderSearchResult, String> {
    with_imap_session!(&config, session => {
        imap_client::search_folder(&mut session, &folder, since_date).await
    })
}

#[tauri::command]
pub async fn imap_sync_folder(
    config: ImapConfig,
    folder: String,
    batch_size: u32,
    since_date: Option<String>,
) -> Result<ImapFolderSyncResult, String> {
    with_imap_session!(&config, session => {
        imap_client::sync_folder(&mut session, &folder, batch_size, since_date).await
    })
}

#[tauri::command]
pub async fn imap_raw_fetch_diagnostic(
    config: ImapConfig,
    folder: String,
    uid_range: String,
) -> Result<String, String> {
    imap_client::raw_fetch_diagnostic(&config, &folder, &uid_range).await
}

#[tauri::command]
pub async fn imap_delta_check(
    config: ImapConfig,
    folders: Vec<DeltaCheckRequest>,
) -> Result<Vec<DeltaCheckResult>, String> {
    with_imap_session!(&config, session => {
        imap_client::delta_check_folders(&mut session, &folders).await
    })
}

// ---------- SMTP commands ----------

#[tauri::command]
pub async fn smtp_send_email(
    config: SmtpConfig,
    raw_email: String,
) -> Result<SmtpSendResult, String> {
    smtp_client::send_raw_email(&config, &raw_email).await
}

#[tauri::command]
pub async fn smtp_test_connection(config: SmtpConfig) -> Result<SmtpSendResult, String> {
    smtp_client::test_connection(&config).await
}
