use super::connection::Db;
use super::types::*;
use crate::ui::thread_list::TypeaheadItem;

impl Db {
    pub fn get_account_auth_info(
        &self,
        account_id: &str,
    ) -> Result<rtsk::db::queries_extra::AccountAuthInfo, String> {
        self.with_conn_sync(|conn| rtsk::db::queries_extra::get_account_auth_info_sync(conn, account_id))
    }

    pub fn get_shared_mailbox_email(
        &self,
        account_id: &str,
        mailbox_id: &str,
    ) -> Result<Option<String>, String> {
        self.with_conn_sync(|conn| {
            rtsk::db::queries_extra::get_shared_mailbox_email_sync(conn, account_id, mailbox_id)
        })
    }

    pub fn get_send_identity_emails_sync(&self) -> Result<Vec<String>, String> {
        self.with_conn_sync(|conn| {
            rtsk::send_identity::get_all_send_identity_emails(conn)
        })
    }

    pub async fn get_accounts(&self) -> Result<Vec<Account>, String> {
        let db = self.read_db_state();
        let rows = rtsk::db::queries_extra::db_get_all_accounts(&db).await?;
        Ok(rows
            .into_iter()
            .map(|row| Account {
                id: row.id,
                email: row.email,
                display_name: row.display_name,
                provider: row.provider,
                account_name: row.account_name,
                account_color: row.account_color,
                last_sync_at: row.last_sync_at,
                token_expires_at: row.token_expires_at,
                is_active: row.is_active != 0,
                sort_order: row.sort_order,
            })
            .collect())
    }

    /// Load all shared mailboxes for sidebar display, across all accounts.
    pub async fn get_shared_mailboxes(&self) -> Result<Vec<SharedMailbox>, String> {
        self.with_conn(|conn| {
            Ok(rtsk::db::queries_extra::get_shared_mailboxes_sync(conn)?
                .into_iter()
                .map(|row| SharedMailbox {
                    mailbox_id: row.mailbox_id,
                    display_name: row.display_name,
                    account_id: row.account_id,
                    is_sync_enabled: row.is_sync_enabled,
                    last_synced_at: row.last_synced_at,
                    sync_error: row.sync_error,
                })
                .collect())
        })
        .await
    }

    /// Load pinned public folders for sidebar display, across all accounts.
    pub async fn get_pinned_public_folders(&self) -> Result<Vec<PinnedPublicFolder>, String> {
        self.with_conn(|conn| {
            Ok(rtsk::db::queries_extra::get_pinned_public_folders_sync(conn)?
                .into_iter()
                .map(|row| PinnedPublicFolder {
                    folder_id: row.folder_id,
                    display_name: row.display_name,
                    account_id: row.account_id,
                    sync_enabled: row.sync_enabled,
                    position: row.position,
                    unread_count: row.unread_count,
                })
                .collect())
        })
        .await
    }

    /// Search labels across all accounts for typeahead suggestions.
    pub async fn search_labels_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            Ok(rtsk::db::queries_extra::search_labels_for_typeahead_sync(conn, &partial)?
                .into_iter()
                .map(|row| TypeaheadItem {
                    label: row.name.clone(),
                    detail: Some(row.account_email),
                    insert_value: row.name,
                })
                .collect())
        })
        .await
    }

    /// Search seen addresses for typeahead suggestions (from:/to: operators).
    pub async fn search_contacts_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            Ok(
                rtsk::db::queries_extra::search_seen_addresses_for_typeahead_sync(conn, &partial)?
                    .into_iter()
                    .map(|row| {
                        let label = row
                            .display_name
                            .as_deref()
                            .unwrap_or(&row.email)
                            .to_string();
                        TypeaheadItem {
                            label,
                            detail: Some(row.email.clone()),
                            insert_value: row.email,
                        }
                    })
                    .collect(),
            )
        })
        .await
    }

    /// Search accounts for typeahead suggestions (account: operator).
    pub async fn search_accounts_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            Ok(
                rtsk::db::queries_extra::search_accounts_for_typeahead_sync(conn, &partial)?
                    .into_iter()
                    .map(|row| {
                        let label = row
                            .account_name
                            .as_deref()
                            .or(row.display_name.as_deref())
                            .unwrap_or(&row.email)
                            .to_string();
                        TypeaheadItem {
                            label: label.clone(),
                            detail: Some(row.email),
                            insert_value: label,
                        }
                    })
                    .collect(),
            )
        })
        .await
    }

    pub async fn any_auto_response_active(&self) -> Result<bool, String> {
        self.read_db_state()
            .with_conn(rtsk::auto_responses::any_auto_response_active)
            .await
    }
}
