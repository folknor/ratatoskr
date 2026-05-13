//! Attachments roadmap Phase 5: Open / Save / Save All handlers.
//!
//! Both surfaces - the reading pane and the pop-out message view -
//! call into these `impl ReadyApp` methods. The reading pane reaches
//! them via `ReadingPaneEvent`; the pop-out handler dispatches them
//! directly (it already lives at the App-method level).
//!
//! All three actions route through `attachment.fetch` IPC: the
//! Service materializes the blob from `PackStore` to
//! `<app_data>/attachment_fetch_tmp/<hash>-<uuid>` and returns the
//! relative path. The UI reads the file positionally. Bytes never
//! cross JSON.
//!
//! Error reporting is log-only in v1 (toast surface is absent per
//! TODO.md). The user sees no feedback on fetch failure beyond the
//! action having no effect. Phase 8's opened-files cleanup pass keeps
//! `<app_data>/opened_attachments/` bounded.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::Task;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::service_client::ServiceClient;

/// Bounded last-folder cache. ~64 entries is plenty for a session;
/// the eviction policy is FIFO by insertion order, which gives
/// "remember the last N threads I saved into." Cross-session
/// persistence is the deferred follow-up; this struct owns the
/// short-lived per-session memory.
const LAST_FOLDER_CACHE_CAP: usize = 64;

#[derive(Debug, Default)]
pub struct LastFolderCache {
    map: std::collections::HashMap<(String, String), PathBuf>,
    order: std::collections::VecDeque<(String, String)>,
}

impl LastFolderCache {
    pub fn new() -> Self { Self::default() }

    pub fn get(&self, key: &(String, String)) -> Option<&PathBuf> {
        self.map.get(key)
    }

    pub fn remember(&mut self, key: (String, String), folder: PathBuf) {
        if self.map.insert(key.clone(), folder).is_none() {
            self.order.push_back(key);
        }
        while self.map.len() > LAST_FOLDER_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

/// A single attachment's identity + display metadata. Shared payload
/// across the three actions so the handler doesn't have to re-query
/// the DB for filenames the UI already had in scope.
#[derive(Debug, Clone)]
pub struct AttachmentRef {
    pub account_id:    String,
    pub message_id:    String,
    pub attachment_id: String,
    pub filename:      Option<String>,
    pub mime_type:     Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpenAttachmentParams {
    pub item: AttachmentRef,
}

#[derive(Debug, Clone)]
pub struct SaveAttachmentParams {
    pub thread_id: String,
    pub item:      AttachmentRef,
}

#[derive(Debug, Clone)]
pub struct SaveAllAttachmentsParams {
    pub thread_id: String,
    pub items:     Vec<AttachmentRef>,
}

impl ReadyApp {
    /// Fetch the attachment, write a copy to
    /// `<app_data>/opened_attachments/<safe_name>`, and shell out to
    /// the OS default handler.
    pub(crate) fn handle_open_attachment(
        &mut self,
        params: OpenAttachmentParams,
    ) -> Task<Message> {
        let Some(client) = self.service_client.clone() else {
            log::warn!("open_attachment: service_client unavailable");
            return Task::none();
        };
        let app_data_dir = match crate::APP_DATA_DIR.get() {
            Some(p) => p.clone(),
            None => {
                log::warn!("open_attachment: APP_DATA_DIR not set");
                return Task::none();
            }
        };
        Task::perform(
            open_attachment_worker(client, app_data_dir, params.item),
            |_| Message::Noop,
        )
    }

    /// Show a Save dialog, fetch the attachment, write to the chosen
    /// path. Remembers the directory per-thread for the session.
    pub(crate) fn handle_save_attachment(
        &mut self,
        params: SaveAttachmentParams,
    ) -> Task<Message> {
        let Some(client) = self.service_client.clone() else {
            log::warn!("save_attachment: service_client unavailable");
            return Task::none();
        };
        let app_data_dir = match crate::APP_DATA_DIR.get() {
            Some(p) => p.clone(),
            None => {
                log::warn!("save_attachment: APP_DATA_DIR not set");
                return Task::none();
            }
        };
        let initial_dir = self
            .attachment_last_folders
            .get(&(params.item.account_id.clone(), params.thread_id.clone()))
            .cloned();
        let folder_key = (params.item.account_id.clone(), params.thread_id.clone());
        Task::perform(
            save_attachment_worker(client, app_data_dir, params.item, initial_dir),
            move |chosen| match chosen {
                Some(parent) => Message::AttachmentSaveFolderRemembered(folder_key.clone(), parent),
                None => Message::Noop,
            },
        )
    }

    /// Pick a folder, fetch every attachment in `items`, write each
    /// with `(N)` collision suffix on name conflicts.
    pub(crate) fn handle_save_all_attachments(
        &mut self,
        params: SaveAllAttachmentsParams,
    ) -> Task<Message> {
        if params.items.is_empty() {
            return Task::none();
        }
        let Some(client) = self.service_client.clone() else {
            log::warn!("save_all_attachments: service_client unavailable");
            return Task::none();
        };
        let app_data_dir = match crate::APP_DATA_DIR.get() {
            Some(p) => p.clone(),
            None => {
                log::warn!("save_all_attachments: APP_DATA_DIR not set");
                return Task::none();
            }
        };
        // params.items is non-empty (guarded above).
        let folder_key = (params.items[0].account_id.clone(), params.thread_id.clone());
        let initial_dir = self.attachment_last_folders.get(&folder_key).cloned();
        Task::perform(
            save_all_attachments_worker(client, app_data_dir, params.items, initial_dir),
            move |chosen| match chosen {
                Some(folder) => Message::AttachmentSaveFolderRemembered(folder_key.clone(), folder),
                None => Message::Noop,
            },
        )
    }
}

async fn open_attachment_worker(
    client:       Arc<ServiceClient>,
    app_data_dir: PathBuf,
    item:         AttachmentRef,
) {
    let ack = match client
        .fetch_attachment(item.account_id, item.message_id, item.attachment_id.clone())
        .await
    {
        Ok(a) => a,
        Err(e) => {
            log::warn!("open_attachment fetch failed: {e}");
            return;
        }
    };
    let src = app_data_dir.join(&ack.relative_path);
    let bytes = match tokio::fs::read(&src).await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("open_attachment read tmp ({}): {e}", src.display());
            return;
        }
    };
    let safe = sanitize_attachment_filename(
        item.filename.as_deref().unwrap_or("attachment"),
    );
    let dest_dir = app_data_dir.join("opened_attachments");
    if let Err(e) = tokio::fs::create_dir_all(&dest_dir).await {
        log::warn!("open_attachment mkdir ({}): {e}", dest_dir.display());
        return;
    }
    // Two opens of different report.pdf attachments must not clobber
    // each other. Reuse the Save All collision-suffix logic.
    let dest = pick_collision_free_path(&dest_dir, &safe);
    if let Err(e) = atomic_write(&dest, &bytes).await {
        log::warn!("open_attachment write ({}): {e}", dest.display());
        return;
    }
    open_file_with_os_default(&dest);
}

/// Returns `Some(parent_dir)` if the user picked a path - the parent
/// is the folder to remember for next time. `None` on cancel or
/// failure.
async fn save_attachment_worker(
    client:       Arc<ServiceClient>,
    app_data_dir: PathBuf,
    item:         AttachmentRef,
    initial_dir:  Option<PathBuf>,
) -> Option<PathBuf> {
    let suggested =
        sanitize_attachment_filename(item.filename.as_deref().unwrap_or("attachment"));
    let (filter_label, filter_ext) = save_dialog_filter(
        item.filename.as_deref(),
        item.mime_type.as_deref(),
    );

    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Save Attachment")
        .set_file_name(&suggested)
        .add_filter(filter_label.as_str(), &[filter_ext.as_str()]);
    if let Some(dir) = initial_dir.as_ref().filter(|p| p.exists()) {
        dialog = dialog.set_directory(dir);
    } else if let Some(dl) = dirs::download_dir() {
        dialog = dialog.set_directory(dl);
    }
    let handle = dialog.save_file().await?;
    let chosen = handle.path().to_path_buf();

    let ack = match client
        .fetch_attachment(item.account_id, item.message_id, item.attachment_id)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            log::warn!("save_attachment fetch failed: {e}");
            return None;
        }
    };
    let src = app_data_dir.join(&ack.relative_path);
    let bytes = match tokio::fs::read(&src).await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("save_attachment read tmp ({}): {e}", src.display());
            return None;
        }
    };
    if let Err(e) = atomic_write(&chosen, &bytes).await {
        log::warn!("save_attachment write ({}): {e}", chosen.display());
        return None;
    }
    chosen.parent().map(Path::to_path_buf)
}

/// Returns `Some(folder)` if the user picked a folder. `None` on
/// cancel.
async fn save_all_attachments_worker(
    client:       Arc<ServiceClient>,
    app_data_dir: PathBuf,
    items:        Vec<AttachmentRef>,
    initial_dir:  Option<PathBuf>,
) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Save All Attachments");
    if let Some(dir) = initial_dir.as_ref().filter(|p| p.exists()) {
        dialog = dialog.set_directory(dir);
    } else if let Some(dl) = dirs::download_dir() {
        dialog = dialog.set_directory(dl);
    }
    let handle = dialog.pick_folder().await?;
    let folder = handle.path().to_path_buf();

    let total = items.len();
    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    for item in items {
        let label = item
            .filename
            .clone()
            .unwrap_or_else(|| item.attachment_id.clone());
        let ack = match client
            .fetch_attachment(
                item.account_id.clone(),
                item.message_id.clone(),
                item.attachment_id.clone(),
            )
            .await
        {
            Ok(a) => a,
            Err(e) => {
                log::warn!("save_all fetch {label}: {e}");
                fail_count += 1;
                continue;
            }
        };
        let src = app_data_dir.join(&ack.relative_path);
        let bytes = match tokio::fs::read(&src).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("save_all read tmp {label}: {e}");
                fail_count += 1;
                continue;
            }
        };
        let safe = sanitize_attachment_filename(item.filename.as_deref().unwrap_or("attachment"));
        let dest = pick_collision_free_path(&folder, &safe);
        if let Err(e) = atomic_write(&dest, &bytes).await {
            log::warn!("save_all write {} ({label}): {e}", dest.display());
            fail_count += 1;
            continue;
        }
        ok_count += 1;
    }
    log::info!("save_all attachments: total={total} ok={ok_count} fail={fail_count}");
    Some(folder)
}

/// Strip path separators, control chars, and Windows-reserved chars
/// from an attachment filename. Preserves dots so the OS default
/// handler can dispatch on extension. Also escapes Windows reserved
/// device names (CON, PRN, NUL, AUX, COM1-9, LPT1-9) by prefixing
/// `_` so a `CON.pdf` attachment can be written on Windows.
pub(crate) fn sanitize_attachment_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let bad = ch == '/'
            || ch == '\\'
            || ch == '<'
            || ch == '>'
            || ch == ':'
            || ch == '"'
            || ch == '|'
            || ch == '?'
            || ch == '*'
            || (ch as u32) < 0x20;
        if bad {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        return "attachment".to_string();
    }
    if is_windows_reserved_device(&trimmed) {
        return format!("_{trimmed}");
    }
    trimmed
}

/// Match against the Windows reserved device names (case-insensitive,
/// matched on the part before any extension). These names cannot be
/// used as filenames on NTFS even when accompanied by an extension.
fn is_windows_reserved_device(name: &str) -> bool {
    let stem = match name.rfind('.') {
        Some(0) => name,
        Some(i) => &name[..i],
        None => name,
    };
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "NUL" | "AUX"
        | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
        | "COM6" | "COM7" | "COM8" | "COM9"
        | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
        | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
}

/// Write `bytes` to `dest` via a sibling tempfile + rename so a crash
/// (or out-of-space) never leaves a partial file at the user's chosen
/// path. The tempfile lives in `dest`'s parent so the rename is on
/// the same filesystem.
async fn atomic_write(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = dest.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination has no parent")
    })?;
    let file_name = dest.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination has no file name")
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = parent.join(tmp_name);
    tokio::fs::write(&tmp_path, bytes).await?;
    match tokio::fs::rename(&tmp_path, dest).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(e)
        }
    }
}

/// Find a non-colliding destination by appending ` (N)` before the
/// extension. `report.pdf` -> `report (1).pdf`. If the input has no
/// extension, the suffix lands at the end.
fn pick_collision_free_path(folder: &Path, filename: &str) -> PathBuf {
    let candidate = folder.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_stem_ext(filename);
    for n in 1..1000 {
        let next = match ext {
            Some(e) => folder.join(format!("{stem} ({n}).{e}")),
            None    => folder.join(format!("{filename} ({n})")),
        };
        if !next.exists() {
            return next;
        }
    }
    // Falling through after 1000 collisions is improbable - return the
    // 1000th candidate and let write fail loudly.
    match ext {
        Some(e) => folder.join(format!("{stem} (1000).{e}")),
        None    => folder.join(format!("{filename} (1000)")),
    }
}

fn split_stem_ext(filename: &str) -> (&str, Option<&str>) {
    match filename.rfind('.') {
        Some(0) => (filename, None), // ".hidden" - treat as stem.
        Some(i) if i + 1 < filename.len() => (&filename[..i], Some(&filename[i + 1..])),
        _ => (filename, None),
    }
}

/// Suggest a (filter_label, primary_ext) pair for the rfd save dialog.
/// Prefers the filename's own extension; falls back to mime mapping.
/// `("All files", "*")` is the catch-all.
fn save_dialog_filter(
    filename:  Option<&str>,
    mime_type: Option<&str>,
) -> (String, String) {
    if let Some(name) = filename
        && let (_, Some(ext)) = split_stem_ext(name)
    {
        let lower = ext.to_ascii_lowercase();
        return (format!("{} (.{lower})", lower.to_ascii_uppercase()), lower);
    }
    if let Some(ext) = mime_type.and_then(mime_to_ext) {
        let lower = ext.to_ascii_lowercase();
        return (format!("{} (.{lower})", lower.to_ascii_uppercase()), lower);
    }
    ("All files".to_string(), "*".to_string())
}

fn mime_to_ext(mime: &str) -> Option<&'static str> {
    match mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "application/pdf"            => Some("pdf"),
        "image/jpeg"                 => Some("jpg"),
        "image/png"                  => Some("png"),
        "image/gif"                  => Some("gif"),
        "image/webp"                 => Some("webp"),
        "image/svg+xml"              => Some("svg"),
        "text/plain"                 => Some("txt"),
        "text/html"                  => Some("html"),
        "text/csv"                   => Some("csv"),
        "application/zip"            => Some("zip"),
        "application/json"           => Some("json"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                                     => Some("docx"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                                     => Some("xlsx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                                     => Some("pptx"),
        _ => None,
    }
}

/// Cross-platform OS-default file opener. Mirrors the URL pattern in
/// `crates/app/src/ui/reading_pane.rs::open_url_in_browser`. No
/// whitelist - the path is constructed from sanitized app-data joins.
fn open_file_with_os_default(path: &Path) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = std::process::Command::new("xdg-open").arg(path).spawn() {
            log::warn!("open_file_with_os_default xdg-open: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = std::process::Command::new("open").arg(path).spawn() {
            log::warn!("open_file_with_os_default open: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        // The empty first arg is `start`'s title slot; without it
        // start would interpret a quoted path as the title.
        if let Err(e) = std::process::Command::new("cmd")
            .args(["/c", "start", "", &path.to_string_lossy()])
            .spawn()
        {
            log::warn!("open_file_with_os_default start: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_preserves_extensions_and_strips_separators() {
        assert_eq!(sanitize_attachment_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_attachment_filename("a/b/c.pdf"), "a_b_c.pdf");
        assert_eq!(sanitize_attachment_filename("a\\b\\c.pdf"), "a_b_c.pdf");
        assert_eq!(sanitize_attachment_filename("with space.pdf"), "with space.pdf");
        assert_eq!(sanitize_attachment_filename("hidden\u{0001}.pdf"), "hidden_.pdf");
    }

    #[test]
    fn sanitizer_strips_windows_reserved() {
        assert_eq!(sanitize_attachment_filename("a<b>c.pdf"), "a_b_c.pdf");
        assert_eq!(sanitize_attachment_filename("q?w*e.pdf"), "q_w_e.pdf");
        assert_eq!(sanitize_attachment_filename(r#"in"out.pdf"#), "in_out.pdf");
    }

    #[test]
    fn sanitizer_fallback_when_empty() {
        assert_eq!(sanitize_attachment_filename(""), "attachment");
        assert_eq!(sanitize_attachment_filename("   "), "attachment");
        assert_eq!(sanitize_attachment_filename("..."), "attachment");
    }

    #[test]
    fn collision_suffix_preserves_extension() {
        let (stem, ext) = split_stem_ext("report.pdf");
        assert_eq!(stem, "report");
        assert_eq!(ext, Some("pdf"));
    }

    #[test]
    fn collision_suffix_handles_no_extension() {
        let (stem, ext) = split_stem_ext("README");
        assert_eq!(stem, "README");
        assert_eq!(ext, None);
    }

    #[test]
    fn collision_suffix_handles_dotfile() {
        let (stem, ext) = split_stem_ext(".hidden");
        assert_eq!(stem, ".hidden");
        assert_eq!(ext, None);
    }

    #[test]
    fn mime_mapping_handles_charset_suffix() {
        assert_eq!(mime_to_ext("text/plain; charset=utf-8"), Some("txt"));
        assert_eq!(mime_to_ext("APPLICATION/PDF"), Some("pdf"));
    }
}
