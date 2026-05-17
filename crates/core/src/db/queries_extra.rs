// Re-export moved query groups from the db crate explicitly. Keeping this
// list named avoids reopening the old all-of-queries-extra writer facade.
pub use db::db::queries_extra::action_helpers;
pub use db::db::queries_extra::action_helpers::*;
pub use db::db::queries_extra::account_sync_writes::*;
pub use db::db::queries_extra::auto_responses;
pub use db::db::queries_extra::auto_responses::*;
pub use db::db::queries_extra::caldav_sync;
pub use db::db::queries_extra::bimi;
pub use db::db::queries_extra::bimi::*;
pub use db::db::queries_extra::caldav_sync::*;
pub use db::db::queries_extra::calendar_contacts_writes::*;
pub use db::db::queries_extra::calendars;
pub use db::db::queries_extra::calendars::*;
pub use db::db::queries_extra::chat;
pub use db::db::queries_extra::chat::*;
pub use db::db::queries_extra::clean_shutdown_cursors::*;
pub use db::db::queries_extra::cloud_attachments;
pub use db::db::queries_extra::cloud_attachments::*;
pub use db::db::queries_extra::command_palette;
pub use db::db::queries_extra::command_palette::*;
pub use db::db::queries_extra::contact_carddav;
pub use db::db::queries_extra::contact_carddav::*;
pub use db::db::queries_extra::contact_groups::*;
pub use db::db::queries_extra::contact_persistence::*;
pub use db::db::queries_extra::contact_photos;
pub use db::db::queries_extra::contact_photos::*;
pub use db::db::queries_extra::contact_search;
pub use db::db::queries_extra::contact_search::*;
pub use db::db::queries_extra::contacts;
pub use db::db::queries_extra::contacts::*;
pub use db::db::queries_extra::draft_lifecycle;
pub use db::db::queries_extra::draft_lifecycle::*;
pub use db::db::queries_extra::email_actions::*;
pub use db::db::queries_extra::extract_reindex::*;
pub use db::db::queries_extra::label_groups::*;
pub use db::db::queries_extra::label_intent::*;
pub use db::db::queries_extra::label_persistence::*;
pub use db::db::queries_extra::load_recent_rule_bundled_threads;
pub use db::db::queries_extra::mdn::*;
pub use db::db::queries_extra::message_membership::*;
pub use db::db::queries_extra::message_persistence::*;
pub use db::db::queries_extra::message_queries;
pub use db::db::queries_extra::message_queries::*;
pub use db::db::queries_extra::provider_sync_writes::*;
pub use db::db::queries_extra::scoped_queries;
pub use db::db::queries_extra::scoped_queries::*;
pub use db::db::queries_extra::search_fallback;
pub use db::db::queries_extra::search_fallback::*;
pub use db::db::queries_extra::send_identity::*;
pub use db::db::queries_extra::thread_detail;
pub use db::db::queries_extra::thread_detail::*;
pub use db::db::queries_extra::thread_persistence::*;
pub use db::db::queries_extra::{
    InsertGmailAccountParams, InsertGraphAccountParams, InsertImapOAuthAccountParams,
    AccountAuthInfo, SaveLocalDraftParams, UpdateAccountParams, account_exists_by_email_sync,
    check_gmail_duplicate_sync, db_get_ai_cache, db_get_all_accounts, db_get_all_signatures,
    db_get_local_draft, db_resolve_signature_for_compose, db_save_local_draft_sync, db_set_ai_cache,
    db_update_scheduled_email_status, db_upsert_writing_style_profile,
    delete_account_orchestrate_sync, finalize_graph_profile_sync, get_account_auth_info_sync,
    get_account_provider_sync, get_account_sync, get_active_account_ids_sync,
    get_all_accounts_sync, get_calendar_default_view_sync, get_stored_graph_client_id_sync,
    get_stored_oauth_credentials_sync, get_used_account_colors_sync, insert_gmail_account_sync,
    insert_graph_account_sync, insert_imap_oauth_account_sync, update_gmail_reauth_tokens_sync,
    update_graph_reauth_tokens_sync,
};

// navigation.rs remains in core: depends on smart_folder::count_smart_folder_unread,
// and smart-folder depends on db (cycle).
pub mod navigation;
pub use navigation::*;
