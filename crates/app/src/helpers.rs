use crate::app::App;
use crate::db::{self, Db, Thread};
use crate::message::Message;
use iced::Task;
use rtsk::db::queries::get_threads_for_bundle;
use rtsk::db::queries_extra::navigation::{
    NavigationState, get_navigation_state, get_shared_mailbox_navigation,
};
use rtsk::db::queries_extra::{
    get_active_account_ids_sync, get_public_folder_items, get_threads_for_shared_mailbox,
    get_threads_scoped,
};
use rtsk::db::types::{AccountScope, DbThread};
use rtsk::generation::{ChatList, GenerationToken, Nav};
use rtsk::scope::ViewScope;
use std::sync::Arc;
use types::{Bundle, FeatureView, SidebarSelection};

impl App {
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
                        let label_id = selection.folder_id_for_thread_query();
                        load_shared_mailbox_threads(db, aid, mid, label_id).await
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
        let db_threads = get_threads_scoped(conn, &scope, label_id.as_deref(), Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();

        // When viewing Drafts, also include local-only drafts
        if label_id.as_deref() == Some("DRAFT") {
            let local =
                rtsk::db::queries_extra::get_local_draft_summaries(conn, &scope, Some(1000), None)?;
            let local_threads: Vec<Thread> =
                local.into_iter().map(local_draft_to_app_thread).collect();
            threads.extend(local_threads);
            // Sort all drafts together by updated_at DESC
            threads.sort_by_key(|t| std::cmp::Reverse(t.last_message_at));
        }

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
        Ok(db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect())
    })
    .await
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn load_public_folder_items_async(
    db: Arc<Db>,
    account_id: String,
    folder_id: String,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let items = get_public_folder_items(conn, &account_id, &folder_id, Some(1000))?;
        Ok(items
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
                is_pinned: false,
                is_muted: false,
                has_attachments: false,
                from_name: item.sender_name,
                from_address: item.sender_email,
                is_local_draft: false,
            })
            .collect())
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
        is_pinned: t.is_pinned,
        is_muted: t.is_muted,
        has_attachments: t.has_attachments,
        from_name: t.from_name,
        from_address: t.from_address,
        is_local_draft: false,
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
        is_pinned: false,
        is_muted: false,
        has_attachments: false,
        from_name: None,
        from_address: d.from_email,
        is_local_draft: true,
    }
}
