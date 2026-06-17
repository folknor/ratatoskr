use common::types::{FolderKind, LabelKind};
use db::db::queries_extra::{
    delete_thread_folder_rows, delete_thread_label_rows, insert_full_thread_folders,
    insert_full_thread_labels, recompute_thread_folders_from_messages,
    recompute_thread_labels_from_messages, replace_message_folder_rows, replace_message_label_rows,
};

pub(crate) fn replace_thread_membership_from_full_coverage(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    folders: &[FolderKind],
    labels: &[LabelKind],
) -> Result<(), String> {
    delete_thread_folder_rows(tx, account_id, thread_id)?;
    insert_full_thread_folders(tx, account_id, thread_id, folders)?;

    delete_thread_label_rows(tx, account_id, thread_id)?;
    insert_full_thread_labels(tx, account_id, thread_id, labels)?;
    db::db::queries_extra::finalize_provider_truth_label_membership(tx, account_id, thread_id)
}

pub(crate) fn replace_message_membership_and_recompute(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    message_id: &str,
    folders: &[FolderKind],
    labels: &[LabelKind],
) -> Result<(), String> {
    replace_message_folder_rows(tx, account_id, message_id, folders)?;
    replace_message_label_rows(tx, account_id, message_id, labels)?;
    recompute_thread_folders_from_messages(tx, account_id, thread_id)?;
    recompute_thread_labels_from_messages(tx, account_id, thread_id)
}

/// Per-message replace for providers whose non-keyword label space is
/// empty (JMAP today: every JMAP label is a keyword tracked in
/// `message_keywords`). Touches `message_folders` only - never
/// `message_labels` - so a future keyword-recompute path cannot be
/// silently wiped by an empty per-message label list.
pub(crate) fn replace_message_folders_and_recompute(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    message_id: &str,
    folders: &[FolderKind],
) -> Result<(), String> {
    replace_message_folder_rows(tx, account_id, message_id, folders)?;
    recompute_thread_folders_from_messages(tx, account_id, thread_id)?;
    recompute_thread_labels_from_messages(tx, account_id, thread_id)
}
