// Re-export moved query groups from the db crate explicitly. Keeping this
// list named avoids reopening the old all-of-queries-extra writer facade.
pub(crate) use db_read::db::queries_extra::action_helpers;
pub(crate) use db_read::db::queries_extra::auto_responses;
pub use db_read::db::queries_extra::calendars;
pub use db_read::db::queries_extra::calendars::*;
pub(crate) use db_read::db::queries_extra::chat;
pub(crate) use db_read::db::queries_extra::command_palette;
pub(crate) use db_read::db::queries_extra::contact_carddav;
pub(crate) use db_read::db::queries_extra::contact_photos;
pub(crate) use db_read::db::queries_extra::contact_search;
pub(crate) use db_read::db::queries_extra::contacts;
pub(crate) use db_read::db::queries_extra::draft_lifecycle;
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
    InsertGmailAccountParams, InsertGraphAccountParams, InsertImapOAuthAccountParams,
    AccountAuthInfo, SaveLocalDraftParams, UpdateAccountParams, account_exists_by_email_sync,
    check_gmail_duplicate_sync, db_get_ai_cache, db_get_all_accounts, db_get_all_signatures,
    db_expand_contact_group_with_names, db_find_contact_group_id_by_name,
    db_find_contact_id_by_email, db_find_group_matching_emails, db_get_local_draft,
    db_resolve_signature_for_compose, db_set_ai_cache,
    db_update_scheduled_email_status, db_upsert_writing_style_profile,
    delete_account_orchestrate_sync, finalize_graph_profile_sync, get_account_auth_info_sync,
    get_account_provider_sync, get_account_sync, get_active_account_ids_sync, get_all_accounts_sync,
    get_calendar_default_view_sync, get_drafts_view, get_public_folder_items,
    get_snoozed_threads, get_starred_threads, get_stored_graph_client_id_sync,
    get_stored_oauth_credentials_sync, get_threads_for_label_group,
    get_threads_for_shared_mailbox, get_threads_for_shared_mailbox_label_group,
    get_threads_for_shared_mailbox_snoozed, get_threads_for_shared_mailbox_starred,
    get_threads_scoped, get_used_account_colors_sync, insert_gmail_account_sync,
    insert_graph_account_sync, insert_imap_oauth_account_sync, load_contacts_for_settings_sync,
    load_group_member_emails_sync, load_groups_for_settings_sync, query_thread_list_decorations,
    update_gmail_reauth_tokens_sync, update_graph_reauth_tokens_sync, LocalDraftSummary,
    MatchedGroup, PublicFolderItem, SendIdentity,
};

// navigation.rs remains in core until the smart-folder count/query glue moves
// into db-read proper.
pub mod navigation;
pub use navigation::*;
