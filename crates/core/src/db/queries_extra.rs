// Re-export moved query groups from the db crate explicitly. Keeping this
// list named avoids reopening the old all-of-queries-extra writer facade.
pub(crate) use db_read::db::queries_extra::auto_responses;
pub use db_read::db::queries_extra::calendars;
pub use db_read::db::queries_extra::calendars::*;
pub(crate) use db_read::db::queries_extra::chat;
pub(crate) use db_read::db::queries_extra::command_palette;
pub(crate) use db_read::db::queries_extra::contact_search;
pub(crate) use db_read::db::queries_extra::contacts;
pub(crate) use db_read::db::queries_extra::label_intent::user_visible_label_group_rendered_fragment;
pub use db_read::db::queries_extra::load_recent_rule_bundled_threads;
pub use db_read::db::queries_extra::message_queries;
pub use db_read::db::queries_extra::message_queries::*;
pub(crate) use db_read::db::queries_extra::scoped_queries;
pub(crate) use db_read::db::queries_extra::search_fallback;
pub(crate) use db_read::db::queries_extra::send_identity::*;
pub use db_read::db::queries_extra::thread_detail;
pub use db_read::db::queries_extra::thread_detail::*;
pub use db_read::db::queries_extra::{
    AccountAuthInfo, SaveLocalDraftParams, UpdateAccountParams, account_exists_by_email_sync,
    check_gmail_duplicate_sync, db_get_all_accounts, db_get_all_signatures,
    db_expand_contact_group_with_names, db_find_contact_group_id_by_name,
    db_find_contact_id_by_email, db_find_group_matching_emails, db_get_local_draft,
    db_resolve_signature_for_compose, get_account_auth_info_sync,
    get_account_provider_sync, get_account_sync, get_active_account_ids_sync, get_all_accounts_sync,
    get_calendar_default_view_sync, get_drafts_view, get_public_folder_items,
    get_snoozed_threads, get_starred_threads, get_threads_for_label_group,
    get_threads_for_shared_mailbox, get_threads_for_shared_mailbox_label_group,
    get_threads_for_shared_mailbox_snoozed, get_threads_for_shared_mailbox_starred,
    get_threads_scoped, get_used_account_colors_sync, load_contacts_for_settings_sync,
    load_group_member_emails_sync, load_groups_for_settings_sync, query_thread_list_decorations,
    LocalDraftSummary, MatchedGroup, PublicFolderItem, SendIdentity,
};

// navigation.rs remains in core until the smart-folder count/query glue moves
// into db-read proper.
pub mod navigation;
pub use navigation::*;
