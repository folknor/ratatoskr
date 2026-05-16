use crate::app::ReadyApp;
use crate::db::{self, Db, Thread};
use crate::message::Message;
use iced::Task;
use rtsk::db::queries::get_threads_for_bundle;
use rtsk::db::queries_extra::navigation::{
    NavigationState, get_navigation_state, get_shared_mailbox_navigation,
};
use rtsk::db::queries_extra::{
    get_active_account_ids_sync, get_public_folder_items, get_snoozed_threads, get_starred_threads,
    get_threads_for_label_group, get_threads_for_shared_mailbox,
    get_threads_for_shared_mailbox_label_group, get_threads_scoped, query_thread_list_decorations,
};
use rtsk::db::types::{AccountScope, DbThread};
use rtsk::generation::{ChatList, GenerationToken, Nav};
use rtsk::scope::ViewScope;
use std::sync::Arc;
use types::{Bundle, FeatureView, SidebarSelection, VirtualView};

impl ReadyApp {
    pub(crate) fn current_scope(&self) -> &ViewScope {
        &self.sidebar.selected_scope
    }

    pub(crate) fn fire_navigation_load(
        &self,
        load_gen: GenerationToken<Nav>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let view_scope = self.sidebar.selected_scope.clone();
        Task::perform(
            async move {
                let r = match &view_scope {
                    ViewScope::SharedMailbox {
                        account_id,
                        mailbox_id,
                    } => {
                        let aid = account_id.clone();
                        let mid = mailbox_id.clone();
                        load_shared_mailbox_navigation(db, aid, mid).await
                    }
                    ViewScope::PublicFolder { account_id, .. } => {
                        // Public folders have no sub-navigation - return
                        // an empty navigation state scoped to the parent account.
                        Ok(NavigationState {
                            scope: AccountScope::Single(account_id.clone()),
                            folders: Vec::new(),
                        })
                    }
                    _ => {
                        let scope = view_scope.to_account_scope().unwrap_or(AccountScope::All);
                        load_navigation(db, scope).await
                    }
                };
                (load_gen, r)
            },
            |(g, result)| Message::NavigationLoaded(g, result),
        )
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn load_threads_for_current_view(
        &self,
        load_gen: GenerationToken<Nav>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let view_scope = self.sidebar.selected_scope.clone();
        let selection = self.sidebar.selection.clone();
        Task::perform(
            async move {
                let r = match &view_scope {
                    ViewScope::SharedMailbox {
                        account_id,
                        mailbox_id,
                    } => {
                        let aid = account_id.clone();
                        let mid = mailbox_id.clone();
                        match &selection {
                            SidebarSelection::LabelGroup(group_id) => {
                                load_shared_mailbox_label_group_threads(
                                    db,
                                    aid,
                                    mid,
                                    group_id.as_i64(),
                                )
                                .await
                            }
                            _ => {
                                // `get_threads_for_shared_mailbox` intercepts
                                // "STARRED" / "SNOOZED" internally to route to
                                // the thread-state boolean columns; everything
                                // else hits `thread_folders`. AllMail is the
                                // unfiltered set (None).
                                let label_id = match &selection {
                                    SidebarSelection::VirtualView(VirtualView::Starred) => {
                                        Some("STARRED".to_string())
                                    }
                                    SidebarSelection::VirtualView(VirtualView::Snoozed) => {
                                        Some("SNOOZED".to_string())
                                    }
                                    SidebarSelection::VirtualView(VirtualView::AllMail) => None,
                                    _ => selection.folder_id_for_thread_query(),
                                };
                                load_shared_mailbox_threads(db, aid, mid, label_id).await
                            }
                        }
                    }
                    ViewScope::PublicFolder {
                        account_id,
                        folder_id,
                    } => {
                        let aid = account_id.clone();
                        let fid = folder_id.clone();
                        load_public_folder_items_async(db, aid, fid).await
                    }
                    _ => {
                        let scope = view_scope.to_account_scope().unwrap_or(AccountScope::All);
                        match &selection {
                            SidebarSelection::Bundle(bundle) => {
                                load_threads_for_bundle_view(db, scope, *bundle).await
                            }
                            SidebarSelection::FeatureView(feature) => {
                                load_threads_for_feature_view(*feature).await
                            }
                            SidebarSelection::LabelGroup(group_id) => {
                                load_threads_for_label_group_view(db, scope, group_id.as_i64())
                                    .await
                            }
                            // Virtual views are not folders; they route to the
                            // helpers that read `threads.is_starred` /
                            // `is_snoozed` / no filter rather than joining
                            // `thread_folders`.
                            SidebarSelection::VirtualView(VirtualView::Starred) => {
                                load_threads_starred(db, scope).await
                            }
                            SidebarSelection::VirtualView(VirtualView::Snoozed) => {
                                load_threads_snoozed(db, scope).await
                            }
                            SidebarSelection::VirtualView(VirtualView::AllMail) => {
                                load_threads_scoped(db, scope, None).await
                            }
                            _ => {
                                let label_id = selection.folder_id_for_thread_query();
                                load_threads_scoped(db, scope, label_id).await
                            }
                        }
                    }
                };
                (load_gen, r)
            },
            |(g, result)| Message::ThreadsLoaded(g, result),
        )
    }

    pub(crate) fn load_navigation_and_threads(&mut self) -> Task<Message> {
        let token = self.nav_generation.next();
        let chat_token = self.chat_list_generation.next();
        Task::batch([
            self.fire_navigation_load(token),
            self.load_threads_for_current_view(token),
            self.fire_chat_contacts_load(chat_token),
        ])
    }

    pub(crate) fn fire_chat_contacts_load(
        &self,
        load_gen: GenerationToken<ChatList>,
    ) -> Task<Message> {
        let db_state = self.db.read_db_state();
        Task::perform(
            async move {
                let r = rtsk::chat::get_chat_contacts(&db_state).await;
                (load_gen, r)
            },
            |(g, result)| Message::ChatContactsLoaded(g, result),
        )
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub(crate) async fn load_accounts(db: Arc<Db>) -> Result<Vec<db::Account>, String> {
    db.get_accounts().await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_navigation(db: Arc<Db>, scope: AccountScope) -> Result<NavigationState, String> {
    db.with_conn(move |conn| get_navigation_state(conn, &scope))
        .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_scoped(
    db: Arc<Db>,
    scope: AccountScope,
    label_id: Option<String>,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        // `label_id` here is a real `folders.id` value (or None for All Mail).
        // Virtual views (Starred / Snoozed) are dispatched upstream because
        // they are backed by `threads.is_starred` / `is_snoozed`, not by
        // `thread_folders` membership.
        let label = label_id.as_deref();
        let db_threads = get_threads_scoped(conn, &scope, label, Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();

        // When viewing Drafts, also include local-only drafts
        if label == Some("DRAFT") {
            let local =
                rtsk::db::queries_extra::get_local_draft_summaries(conn, &scope, Some(1000), None)?;
            let local_threads: Vec<Thread> =
                local.into_iter().map(local_draft_to_app_thread).collect();
            threads.extend(local_threads);
            // Sort all drafts together by updated_at DESC
            threads.sort_by_key(|t| std::cmp::Reverse(t.last_message_at));
        }

        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_for_label_group_view(
    db: Arc<Db>,
    scope: AccountScope,
    group_id: i64,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads = get_threads_for_label_group(conn, &scope, group_id, Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_starred(
    db: Arc<Db>,
    scope: AccountScope,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads = get_starred_threads(conn, &scope, Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_snoozed(
    db: Arc<Db>,
    scope: AccountScope,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads = get_snoozed_threads(conn, &scope, Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_for_bundle_view(
    db: Arc<Db>,
    scope: AccountScope,
    bundle: Bundle,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let bundle_name = match bundle {
            Bundle::Primary => "Primary",
            Bundle::Updates => "Updates",
            Bundle::Promotions => "Promotions",
            Bundle::Social => "Social",
            Bundle::Newsletters => "Newsletters",
        };

        let account_ids: Vec<String> = match &scope {
            AccountScope::Single(id) => vec![id.clone()],
            AccountScope::Multiple(ids) => ids.clone(),
            AccountScope::All => get_active_account_ids_sync(conn)?,
        };

        let mut threads = Vec::new();
        for account_id in &account_ids {
            let db_threads =
                get_threads_for_bundle(conn, account_id, bundle_name, Some(1000), None)?;
            threads.extend(db_threads.into_iter().map(db_thread_to_app_thread));
        }

        threads.sort_by_key(|t| std::cmp::Reverse(t.last_message_at));
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_threads_for_feature_view(feature: FeatureView) -> Result<Vec<Thread>, String> {
    match feature {
        // These sidebar destinations do not map to the mail thread list yet.
        FeatureView::Tasks | FeatureView::Attachments => Ok(Vec::new()),
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_shared_mailbox_navigation(
    db: Arc<Db>,
    account_id: String,
    mailbox_id: String,
) -> Result<NavigationState, String> {
    db.with_conn(move |conn| get_shared_mailbox_navigation(conn, &account_id, &mailbox_id))
        .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_shared_mailbox_threads(
    db: Arc<Db>,
    account_id: String,
    mailbox_id: String,
    label_id: Option<String>,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads = get_threads_for_shared_mailbox(
            conn,
            &account_id,
            &mailbox_id,
            label_id.as_deref(),
            Some(1000),
        )?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_shared_mailbox_label_group_threads(
    db: Arc<Db>,
    account_id: String,
    mailbox_id: String,
    group_id: i64,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads = get_threads_for_shared_mailbox_label_group(
            conn,
            &account_id,
            &mailbox_id,
            group_id,
            Some(1000),
        )?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

pub(crate) fn apply_thread_decorations(
    conn: &rtsk::db::Connection,
    threads: &mut [Thread],
) -> Result<(), String> {
    let keys: Vec<(String, String)> = threads
        .iter()
        .map(|thread| (thread.account_id.clone(), thread.id.clone()))
        .collect();
    let decorations = query_thread_list_decorations(conn, &keys)?;
    let by_key: std::collections::HashMap<(String, String), _> = decorations
        .into_iter()
        .map(|decoration| {
            (
                (decoration.account_id.clone(), decoration.thread_id.clone()),
                decoration,
            )
        })
        .collect();

    for thread in threads {
        if let Some(decoration) = by_key.get(&(thread.account_id.clone(), thread.id.clone())) {
            thread.is_replied = decoration.is_replied;
            thread.is_forwarded = decoration.is_forwarded;
            thread.label_color_bgs = decoration.label_color_bgs.clone();
        }
    }
    Ok(())
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_public_folder_items_async(
    db: Arc<Db>,
    account_id: String,
    folder_id: String,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let items = get_public_folder_items(conn, &account_id, &folder_id, Some(1000))?;
        let mut threads: Vec<Thread> = items
            .into_iter()
            .map(|item| Thread {
                id: item.item_id,
                account_id: item.account_id,
                subject: item.subject,
                snippet: item.body_preview,
                last_message_at: item.received_at,
                message_count: 1,
                is_read: item.is_read,
                is_starred: false,
                is_replied: false,
                is_forwarded: false,
                is_pinned: false,
                is_muted: false,
                has_attachments: false,
                label_color_bgs: Vec::new(),
                from_name: item.sender_name,
                from_address: item.sender_email,
                is_local_draft: false,
                match_kind: None,
                also_matched: Vec::new(),
            })
            .collect();
        apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

pub(crate) fn db_thread_to_app_thread(t: DbThread) -> Thread {
    Thread {
        id: t.id,
        account_id: t.account_id,
        subject: t.subject,
        snippet: t.snippet,
        last_message_at: t.last_message_at,
        message_count: t.message_count,
        is_read: t.is_read,
        is_starred: t.is_starred,
        is_replied: false,
        is_forwarded: false,
        is_pinned: t.is_pinned,
        is_muted: t.is_muted,
        has_attachments: t.has_attachments,
        label_color_bgs: Vec::new(),
        from_name: t.from_name,
        from_address: t.from_address,
        is_local_draft: false,
        match_kind: None,
        also_matched: Vec::new(),
    }
}

pub(crate) fn local_draft_to_app_thread(d: rtsk::db::queries_extra::LocalDraftSummary) -> Thread {
    Thread {
        id: d.id,
        account_id: d.account_id,
        subject: d.subject,
        snippet: d.snippet,
        last_message_at: Some(d.updated_at),
        message_count: 1,
        is_read: true,
        is_starred: false,
        is_replied: false,
        is_forwarded: false,
        is_pinned: false,
        is_muted: false,
        has_attachments: false,
        label_color_bgs: Vec::new(),
        from_name: None,
        from_address: d.from_email,
        is_local_draft: true,
        match_kind: None,
        also_matched: Vec::new(),
    }
}
