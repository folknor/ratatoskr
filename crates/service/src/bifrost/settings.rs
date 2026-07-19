//! Account-settings forwarding through the resident bifrost engine.

use bifrost_types::AccountId;
pub(crate) use bifrost_types::{Identity, IdentityId, IdentityPatch, QuotaInfo, VacationConfig};

use super::ResidentEngine;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AccountSettingsError {
    #[error("resident attach failed: {0}")]
    Attach(String),
    #[error("account-settings engine call failed: {0}")]
    Engine(#[source] bifrost_sync::Error),
}

/// Capability-dispatched account-settings surface. The resident handle attaches
/// cold accounts before forwarding.
#[derive(Clone)]
pub(crate) struct AccountSettingsSurface {
    resident: ResidentEngine,
}

impl AccountSettingsSurface {
    pub(crate) fn new(resident: ResidentEngine) -> Self {
        Self { resident }
    }

    pub(crate) async fn identities(
        &self,
        account_id: &str,
    ) -> Result<Vec<Identity>, AccountSettingsError> {
        let action_account = self
            .resident
            .attached_action_account(account_id)
            .await
            .map_err(AccountSettingsError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .identities_list(&account)
            .await
            .map_err(AccountSettingsError::Engine)
    }

    pub(crate) async fn identity_update(
        &self,
        account_id: &str,
        identity: IdentityId,
        patch: IdentityPatch,
    ) -> Result<(), AccountSettingsError> {
        let action_account = self
            .resident
            .attached_action_account(account_id)
            .await
            .map_err(AccountSettingsError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .identity_update(&account, identity, patch)
            .await
            .map_err(AccountSettingsError::Engine)
    }

    #[allow(dead_code)] // pinned vacation surface; B13 adds no UI caller
    pub(crate) async fn vacation_get(
        &self,
        account_id: &str,
    ) -> Result<Option<VacationConfig>, AccountSettingsError> {
        let action_account = self
            .resident
            .attached_action_account(account_id)
            .await
            .map_err(AccountSettingsError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .vacation_get(&account)
            .await
            .map_err(AccountSettingsError::Engine)
    }

    #[allow(dead_code)] // pinned vacation surface; B13 adds no UI caller
    pub(crate) async fn vacation_set(
        &self,
        account_id: &str,
        config: VacationConfig,
    ) -> Result<(), AccountSettingsError> {
        let action_account = self
            .resident
            .attached_action_account(account_id)
            .await
            .map_err(AccountSettingsError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .vacation_set(&account, config)
            .await
            .map_err(AccountSettingsError::Engine)
    }

    #[allow(dead_code)] // pinned quota surface; B13 adds no UI caller
    pub(crate) async fn quota_get(
        &self,
        account_id: &str,
    ) -> Result<Option<QuotaInfo>, AccountSettingsError> {
        let action_account = self
            .resident
            .attached_action_account(account_id)
            .await
            .map_err(AccountSettingsError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .quota_get(&account)
            .await
            .map_err(AccountSettingsError::Engine)
    }
}
