//! Read-only façade over the writer-side `db` crate.
//!
//! This crate exists so `rtsk` and (transitively) `app` can name the
//! database's *read* surface (`ReadConn`, `ReadStatement`,
//! `ReadDbState`, the read-only query modules) without naming the
//! writer's `Connection`, `WriteConn`, `WriterPool`, or any other
//! mutating handle.
//!
//! Every re-export below is named explicitly. **No glob re-exports**:
//! `pub use writer_db::db::*` would pull mutating rusqlite types and
//! the writer pool through `db-read`'s surface, defeating the brokkr
//! `core-no-writer-db` rule that lets `rtsk` depend on `db-read`
//! without depending on the writer crate. The
//! `db_read_public_surface_does_not_reexport_rusqlite` lockdown test
//! pins this invariant; the
//! `db_read_raw_rusqlite_access_is_quarantined` test pins the
//! file-level discipline inside this crate.
//!
//! The clean `ReadConn` / `ReadStatement` / `ReadDbState`
//! implementations live in `writer_db::db` so there is exactly one
//! type underlying both this crate's `db_read::ReadConn` and the
//! writer-side `db::db::ReadConn`. The previous design had two
//! parallel structs (a clean one in `raw.rs` here, a buggy bridge in
//! `writer_db::db`); the trybuild lockdown verified the clean type
//! while every production caller resolved through the bridge.

pub use writer_db::{blob_hash, impl_from_row, impl_from_row_munch, progress};
pub use writer_db::db::{
    ReadCachedStatement, ReadConn, ReadDbState, ReadError, ReadStatement,
};

pub mod db {
    //! Mirror of `writer_db::db` restricted to the read-safe surface.
    //!
    //! Items deliberately absent: `WriteConn`, `WriteTxn`,
    //! `WriterPool`, `apply_writer_pragmas`, `reconcile_velo_rename`,
    //! `open_writer_pool`. Naming any of those from a `db-read`
    //! consumer must fail to resolve.
    //!
    pub use writer_db::db::{
        // Constants
        DEFAULT_QUERY_LIMIT,
        // Read-side connection wrappers (single canonical types,
        // defined in writer_db::db; re-exported here so existing
        // `rtsk::db::db::ReadConn` paths keep resolving).
        ReadCachedStatement,
        ReadConn,
        ReadDbState,
        ReadError,
        ReadStatement,
        // Reader pool entry point. The writer pool entry
        // (`open_writer_pool`) is intentionally NOT re-exported.
        open_reader_pool,
        // Reader-safe pragma application. The writer counterpart is
        // intentionally NOT re-exported.
        apply_reader_pragmas,
        // rusqlite passthroughs that read code names by type.
        OptionalExtension,
        Row,
        SqlError,
        ToSql,
        params,
        // FromRow trait + helpers, used by every read query that
        // shapes rows into typed structs.
        FromRow,
        from_row,
        query_as,
        query_one,
        // Utility modules safe for read use.
        folder_roles,
        lookups,
        pinned_searches,
        sql_fragments,
        types,
    };

    pub mod queries {
        pub use writer_db::db::queries::{
            get_attachments_for_message, get_bundle_unread_counts, get_categories_for_threads,
            get_contact_by_email, get_folders, get_labels, get_provider_type, get_setting,
            get_thread_by_id, get_thread_count, get_thread_folder_ids, get_thread_label_ids,
            get_threads, get_threads_for_bundle, get_unread_count, search_contacts,
        };
    }

    pub mod queries_extra {
        pub use writer_db::db::queries_extra::action_helpers;
        pub use writer_db::db::queries_extra::auto_responses;
        pub use writer_db::db::queries_extra::calendars;
        pub use writer_db::db::queries_extra::calendars::*;
        pub use writer_db::db::queries_extra::chat;
        pub use writer_db::db::queries_extra::command_palette;
        pub use writer_db::db::queries_extra::contact_carddav;
        pub use writer_db::db::queries_extra::contact_photos;
        pub use writer_db::db::queries_extra::contact_search;
        pub use writer_db::db::queries_extra::contacts;
        pub use writer_db::db::queries_extra::draft_lifecycle;
        pub use writer_db::db::queries_extra::extract_reindex;
        pub use writer_db::db::queries_extra::label_intent;
        pub use writer_db::db::queries_extra::label_intent::user_visible_label_group_rendered_fragment;
        pub use writer_db::db::queries_extra::message_queries;
        pub use writer_db::db::queries_extra::message_queries::*;
        pub use writer_db::db::queries_extra::scoped_queries;
        pub use writer_db::db::queries_extra::search_fallback;
        pub use writer_db::db::queries_extra::send_identity;
        pub use writer_db::db::queries_extra::send_identity::*;
        pub use writer_db::db::queries_extra::thread_detail;
        pub use writer_db::db::queries_extra::thread_detail::*;
        pub use writer_db::db::queries_extra::{
            AccountAuthInfo, InsertGmailAccountParams, InsertGraphAccountParams,
            InsertImapOAuthAccountParams, LocalDraftSummary, MatchedGroup, PublicFolderItem,
            SaveLocalDraftParams, SendIdentity, UpdateAccountParams, account_exists_by_email_sync,
            check_gmail_duplicate_sync, db_expand_contact_group_with_names,
            db_find_contact_group_id_by_name, db_find_contact_id_by_email,
            db_find_group_matching_emails, db_get_ai_cache, db_get_all_accounts,
            db_get_all_signatures, db_get_local_draft, db_resolve_signature_for_compose,
            db_set_ai_cache, db_update_scheduled_email_status,
            db_upsert_writing_style_profile, delete_account_orchestrate_sync,
            finalize_graph_profile_sync, get_account_auth_info_sync, get_account_provider_sync,
            get_account_sync, get_active_account_ids_sync, get_all_accounts_sync,
            get_calendar_default_view_sync, get_drafts_view, get_public_folder_items,
            get_snoozed_threads, get_starred_threads, get_stored_graph_client_id_sync,
            get_stored_oauth_credentials_sync, get_threads_for_label_group,
            get_threads_for_shared_mailbox, get_threads_for_shared_mailbox_label_group,
            get_threads_for_shared_mailbox_snoozed, get_threads_for_shared_mailbox_starred,
            get_threads_scoped, get_used_account_colors_sync, insert_gmail_account_sync,
            insert_graph_account_sync, insert_imap_oauth_account_sync,
            load_contacts_for_settings_sync, load_group_member_emails_sync,
            load_groups_for_settings_sync, load_recent_rule_bundled_threads,
            query_thread_list_decorations, select_attachment_fragments_batch,
            update_gmail_reauth_tokens_sync, update_graph_reauth_tokens_sync,
        };
    }
}
