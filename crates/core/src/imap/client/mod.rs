mod commands;
mod sync;

use std::time::Duration;

use async_imap::types::Flag;
use base64::Engine;
use futures::StreamExt;
use mail_parser::MessageParser;

use super::connection::{IMAP_CMD_TIMEOUT, IMAP_FETCH_TIMEOUT, ImapSession};
use super::parse::{build_imap_section_map, detect_special_use, parse_message};
use super::types::*;

// Re-export submodule items
pub use commands::*;
pub use sync::*;

// Re-export items that commands.rs imports via `imap::client::*`
pub use super::connection::connect;
pub use super::raw::{raw_fetch_diagnostic, raw_fetch_messages};

/// Check whether a mailbox's PERMANENTFLAGS includes `\*` (Flag::MayCreate),
/// indicating that the server allows clients to define arbitrary custom keywords.
pub(crate) fn mailbox_supports_custom_keywords(mailbox: &async_imap::types::Mailbox) -> bool {
    mailbox
        .permanent_flags
        .iter()
        .any(|f| matches!(f, Flag::MayCreate))
}

/// Build a standardised timeout error message for IMAP operations.
pub(crate) fn timeout_err(operation: &str, timeout: Duration) -> String {
    format!(
        "{operation} timed out after {}s — check your server settings or network connection",
        timeout.as_secs()
    )
}

/// List all IMAP folders/mailboxes.
pub async fn list_folders(session: &mut ImapSession) -> Result<Vec<ImapFolder>, String> {
    let names_stream = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.list(Some(""), Some("*")))
        .await
        .map_err(|_| timeout_err("LIST", IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("LIST failed: {e}"))?;

    let names: Vec<_> = tokio::time::timeout(IMAP_CMD_TIMEOUT, names_stream.collect::<Vec<_>>())
        .await
        .map_err(|_| timeout_err("LIST stream", IMAP_CMD_TIMEOUT))?
        .into_iter()
        .filter_map(Result::ok)
        .collect();

    let mut folders = Vec::new();
    for name in &names {
        let raw_path = name.name().to_string();
        let delimiter = name.delimiter().unwrap_or("/").to_string();

        // Decode modified UTF-7 (RFC 3501 §5.1.3) to UTF-8 for display
        let path = utf7_imap::decode_utf7_imap(raw_path.clone());

        // Extract display name (last segment after delimiter)
        let display_name = path
            .rsplit_once(&delimiter)
            .map(|(_, last)| last.to_string())
            .unwrap_or_else(|| path.clone());

        // Skip non-selectable container folders (e.g. [Gmail], [Google Mail])
        if name
            .attributes()
            .iter()
            .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
        {
            continue;
        }

        // Detect special-use from attributes (RFC 6154)
        let special_use = detect_special_use(name);

        // Get message counts via STATUS — use raw_path for IMAP commands
        let (exists, unseen) = match tokio::time::timeout(
            IMAP_CMD_TIMEOUT,
            session.status(&raw_path, "(MESSAGES UNSEEN)"),
        )
        .await
        {
            Ok(Ok(mailbox)) => (mailbox.exists, mailbox.unseen.unwrap_or(0)),
            _ => (0, 0),
        };

        folders.push(ImapFolder {
            path,
            raw_path,
            name: display_name,
            delimiter,
            special_use,
            exists,
            unseen,
            namespace_type: None,
        });
    }

    Ok(folders)
}

/// List shared and other-users folders discovered via NAMESPACE.
///
/// For each entry in `namespace_info.other_users` and `namespace_info.shared`,
/// sends `LIST "" "{prefix}*"` to discover all folders under that namespace.
/// Each folder is tagged with its `NamespaceType`.
pub async fn list_shared_folders(
    session: &mut ImapSession,
    namespace_info: &NamespaceInfo,
) -> Result<Vec<ImapFolder>, String> {
    let mut folders = Vec::new();

    // Collect (prefix, namespace_type) pairs to query
    let mut queries: Vec<(&str, NamespaceType)> = Vec::new();
    for entry in &namespace_info.other_users {
        queries.push((&entry.prefix, NamespaceType::OtherUsers));
    }
    for entry in &namespace_info.shared {
        queries.push((&entry.prefix, NamespaceType::Shared));
    }

    for (prefix, ns_type) in &queries {
        if prefix.is_empty() {
            continue;
        }

        let pattern = format!("{prefix}*");
        let names_stream =
            tokio::time::timeout(IMAP_CMD_TIMEOUT, session.list(Some(""), Some(&pattern)))
                .await
                .map_err(|_| timeout_err(&format!("LIST {pattern}"), IMAP_CMD_TIMEOUT))?
                .map_err(|e| format!("LIST {pattern} failed: {e}"))?;

        let names: Vec<_> =
            tokio::time::timeout(IMAP_CMD_TIMEOUT, names_stream.collect::<Vec<_>>())
                .await
                .map_err(|_| timeout_err(&format!("LIST stream {pattern}"), IMAP_CMD_TIMEOUT))?
                .into_iter()
                .filter_map(Result::ok)
                .collect();

        for name in &names {
            let raw_path = name.name().to_string();
            let delimiter = name.delimiter().unwrap_or("/").to_string();
            let path = utf7_imap::decode_utf7_imap(raw_path.clone());
            let display_name = path
                .rsplit_once(&delimiter)
                .map(|(_, last)| last.to_string())
                .unwrap_or_else(|| path.clone());

            // Skip non-selectable container folders
            if name
                .attributes()
                .iter()
                .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
            {
                continue;
            }

            let special_use = detect_special_use(name);

            // Get message counts — use raw_path for IMAP commands
            let (exists, unseen) = match tokio::time::timeout(
                IMAP_CMD_TIMEOUT,
                session.status(&raw_path, "(MESSAGES UNSEEN)"),
            )
            .await
            {
                Ok(Ok(mailbox)) => (mailbox.exists, mailbox.unseen.unwrap_or(0)),
                _ => (0, 0),
            };

            folders.push(ImapFolder {
                path,
                raw_path,
                name: display_name,
                delimiter,
                special_use,
                exists,
                unseen,
                namespace_type: Some(ns_type.clone()),
            });
        }

        log::info!(
            "IMAP LIST shared folders under \"{prefix}\": found {} selectable folders",
            folders.len()
        );
    }

    Ok(folders)
}

/// Fetch messages from a folder by UID range (e.g. "1:100" or "500:*").
pub async fn fetch_messages(
    session: &mut ImapSession,
    folder: &str,
    uid_range: &str,
) -> Result<ImapFetchResult, String> {
    let mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let folder_status = ImapFolderStatus {
        uidvalidity: mailbox.uid_validity.unwrap_or(0),
        uidnext: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        unseen: mailbox.unseen.unwrap_or(0),
        highest_modseq: mailbox.highest_modseq,
        supports_custom_keywords: mailbox_supports_custom_keywords(&mailbox),
    };

    log::info!(
        "IMAP SELECT {folder}: exists={}, uidvalidity={}, uidnext={}, fetching UIDs: {uid_range}",
        mailbox.exists,
        mailbox.uid_validity.unwrap_or(0),
        mailbox.uid_next.unwrap_or(0),
    );

    // Try UID FETCH first; if the stream is empty, fall back to sequence-number FETCH.
    // Some IMAP servers return empty streams for UID FETCH despite valid UIDs.
    let fetches = tokio::time::timeout(IMAP_FETCH_TIMEOUT, async {
        let stream = session
            .uid_fetch(uid_range, "UID FLAGS INTERNALDATE BODY.PEEK[]")
            .await
            .map_err(|e| format!("UID FETCH {folder} uids={uid_range} failed: {e}"))?;
        Ok::<_, String>(stream.collect::<Vec<_>>().await)
    })
    .await
    .map_err(|_| timeout_err(&format!("UID FETCH {folder}"), IMAP_FETCH_TIMEOUT))?;

    let raw_fetches: Vec<_> = fetches?;
    let mut fetch_ok = 0u32;
    let mut fetch_err = 0u32;
    let mut fetches = Vec::new();
    for r in raw_fetches {
        match r {
            Ok(f) => {
                fetch_ok += 1;
                fetches.push(f);
            }
            Err(e) => {
                fetch_err += 1;
                log::warn!("IMAP fetch stream error in {folder}: {e}");
            }
        }
    }
    log::info!("IMAP FETCH {folder}: {fetch_ok} ok, {fetch_err} errors from uid_fetch");

    // If async-imap returned nothing but messages exist, fallback to raw TCP fetch
    if fetches.is_empty() && mailbox.exists > 0 {
        log::warn!(
            "IMAP {folder}: async-imap returned 0 items but exists={}. Falling back to raw TCP fetch...",
            mailbox.exists
        );
        // Return early with raw fetch result — caller doesn't need to know about the fallback
        return Err(format!("ASYNC_IMAP_EMPTY:{folder}"));
    }

    let parser = MessageParser::default();
    let mut messages = Vec::new();
    for fetch in &fetches {
        let uid = match fetch.uid {
            Some(u) => u,
            None => {
                log::warn!("IMAP FETCH {folder}: response missing UID");
                continue;
            }
        };

        let raw = match fetch.body() {
            Some(b) => b,
            None => {
                log::warn!("IMAP FETCH {folder}: UID {uid} has no body");
                continue;
            }
        };

        #[allow(clippy::cast_possible_truncation)]
        let raw_size = raw.len() as u32;

        // Parse flags
        let flags: Vec<_> = fetch.flags().collect();
        let is_read = flags.iter().any(|f| matches!(f, Flag::Seen));
        let is_starred = flags.iter().any(|f| matches!(f, Flag::Flagged));
        let is_draft = flags.iter().any(|f| matches!(f, Flag::Draft));

        // Extract INTERNALDATE as fallback for messages with unparseable Date headers
        let internal_date = fetch.internal_date().map(|dt| dt.timestamp());

        match parse_message(
            &parser,
            raw,
            uid,
            folder,
            raw_size,
            is_read,
            is_starred,
            is_draft,
            internal_date,
        ) {
            Ok(msg) => messages.push(msg),
            Err(e) => {
                log::warn!("Failed to parse message UID {uid}: {e}");
            }
        }
    }

    Ok(ImapFetchResult {
        messages,
        folder_status,
    })
}

/// Fetch a single message body by UID.
pub async fn fetch_message_body(
    session: &mut ImapSession,
    folder: &str,
    uid: u32,
) -> Result<ImapMessage, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let uid_str = uid.to_string();
    let fetches: Vec<_> = tokio::time::timeout(IMAP_FETCH_TIMEOUT, async {
        let stream = session
            .uid_fetch(&uid_str, "UID FLAGS BODY.PEEK[]")
            .await
            .map_err(|e| format!("UID FETCH failed: {e}"))?;
        Ok::<_, String>(stream.collect::<Vec<_>>().await)
    })
    .await
    .map_err(|_| timeout_err(&format!("UID FETCH for UID {uid}"), IMAP_FETCH_TIMEOUT))??
    .into_iter()
    .filter_map(Result::ok)
    .collect();

    let fetch = fetches
        .first()
        .ok_or_else(|| format!("Message UID {uid} not found in {folder}"))?;

    let raw = fetch
        .body()
        .ok_or_else(|| format!("No body for UID {uid}"))?;

    #[allow(clippy::cast_possible_truncation)]
    let raw_size = raw.len() as u32;
    let flags: Vec<_> = fetch.flags().collect();
    let is_read = flags.iter().any(|f| matches!(f, Flag::Seen));
    let is_starred = flags.iter().any(|f| matches!(f, Flag::Flagged));
    let is_draft = flags.iter().any(|f| matches!(f, Flag::Draft));

    let parser = MessageParser::default();
    parse_message(
        &parser, raw, uid, folder, raw_size, is_read, is_starred, is_draft, None,
    )
}

/// Fetch a specific MIME part (attachment) by UID and part ID.
/// Returns the decoded binary data as standard base64.
///
/// Fetches the full message via `BODY.PEEK[]`, parses it with `mail-parser`
/// (which handles all content-transfer-encoding decoding), and extracts
/// the requested part's decoded bytes.
pub async fn fetch_attachment(
    session: &mut ImapSession,
    folder: &str,
    uid: u32,
    part_id: &str,
) -> Result<String, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let uid_str = uid.to_string();
    let fetches: Vec<_> = tokio::time::timeout(IMAP_FETCH_TIMEOUT, async {
        let stream = session
            .uid_fetch(&uid_str, "BODY.PEEK[]")
            .await
            .map_err(|e| format!("UID FETCH attachment failed: {e}"))?;
        Ok::<_, String>(stream.collect::<Vec<_>>().await)
    })
    .await
    .map_err(|_| timeout_err("UID FETCH attachment", IMAP_FETCH_TIMEOUT))??
    .into_iter()
    .filter_map(Result::ok)
    .collect();

    let fetch = fetches
        .first()
        .ok_or_else(|| format!("No response for UID {uid}"))?;

    let raw = fetch
        .body()
        .ok_or_else(|| format!("No body for UID {uid}"))?;

    // Parse the full message — mail-parser decodes content-transfer-encoding
    let parser = MessageParser::default();
    let message = parser
        .parse(raw)
        .ok_or_else(|| format!("Failed to parse message UID {uid}"))?;

    // Build section map and find the part index for the requested section path
    let section_map = build_imap_section_map(&message);
    let target_part_idx = section_map
        .iter()
        .find(|(_, section)| section.as_str() == part_id)
        .map(|(&idx, _)| idx)
        .ok_or_else(|| format!("Section {part_id} not found in message UID {uid}"))?;

    let part = message
        .parts
        .get(target_part_idx)
        .ok_or_else(|| format!("Part index {target_part_idx} out of range for UID {uid}"))?;

    // Extract the decoded binary content from the part
    let data = match &part.body {
        mail_parser::PartType::Binary(data) | mail_parser::PartType::InlineBinary(data) => {
            data.as_ref().to_vec()
        }
        mail_parser::PartType::Text(text) => text.as_bytes().to_vec(),
        mail_parser::PartType::Html(html) => html.as_bytes().to_vec(),
        mail_parser::PartType::Message(msg) => {
            // Nested message — encode the raw bytes
            msg.raw_message.as_ref().to_vec()
        }
        mail_parser::PartType::Multipart(_) => {
            return Err(format!(
                "Part {part_id} is a multipart container, not a leaf part"
            ));
        }
    };

    Ok(base64::engine::general_purpose::STANDARD.encode(&data))
}

/// Fetch the raw RFC822 source of a single message by UID.
/// Returns the full message as a UTF-8 string (lossy conversion for non-UTF-8 bytes).
pub async fn fetch_raw_message(
    session: &mut ImapSession,
    folder: &str,
    uid: u32,
) -> Result<String, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let uid_str = uid.to_string();
    let fetches: Vec<_> = tokio::time::timeout(IMAP_FETCH_TIMEOUT, async {
        let stream = session
            .uid_fetch(&uid_str, "BODY.PEEK[]")
            .await
            .map_err(|e| format!("UID FETCH failed: {e}"))?;
        Ok::<_, String>(stream.collect::<Vec<_>>().await)
    })
    .await
    .map_err(|_| timeout_err("UID FETCH raw message", IMAP_FETCH_TIMEOUT))??
    .into_iter()
    .filter_map(Result::ok)
    .collect();

    let fetch = fetches
        .first()
        .ok_or_else(|| format!("Message UID {uid} not found in {folder}"))?;

    let raw = fetch
        .body()
        .ok_or_else(|| format!("No body for UID {uid}"))?;

    Ok(String::from_utf8_lossy(raw).to_string())
}

/// Test IMAP connectivity: connect, login, list, logout.
pub async fn test_connection(config: &ImapConfig) -> Result<String, String> {
    let mut session = connect(config).await?;

    // Try listing folders to verify access
    let count = tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let names = session
            .list(Some(""), Some("*"))
            .await
            .map_err(|e| format!("LIST failed: {e}"))?;
        Ok::<_, String>(names.collect::<Vec<_>>().await.len())
    })
    .await
    .map_err(|_| timeout_err("LIST", IMAP_CMD_TIMEOUT))??;

    _ = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.logout()).await;

    Ok(format!("Connected successfully. Found {count} folder(s)."))
}
