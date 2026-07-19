//! Server-side filter forwarding through the resident bifrost engine.

use bifrost_types::AccountId;
pub(crate) use bifrost_types::{
    FilterValidation, ServerFilter, ServerFilterCreate, ServerFilterId, ServerFilterPatch,
};

use super::ResidentEngine;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ServerFilterError {
    #[error("resident attach failed: {0}")]
    Attach(String),
    #[error("server-filter engine call failed: {0}")]
    Engine(#[source] bifrost_sync::Error),
}

/// Capability-dispatched server-filter surface reserved for a future settings
/// caller. The resident handle attaches cold accounts before forwarding.
#[derive(Clone)]
#[allow(dead_code)] // pinned filters-settings surface; B11 adds no UI caller
pub(crate) struct ServerFilterSurface {
    resident: ResidentEngine,
}

#[allow(dead_code)] // pinned filters-settings surface; B11 adds no UI caller
impl ServerFilterSurface {
    pub(crate) fn new(resident: ResidentEngine) -> Self {
        Self { resident }
    }

    pub(crate) async fn list(
        &self,
        account_id: &str,
    ) -> Result<Vec<ServerFilter>, ServerFilterError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(ServerFilterError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .filters_list(&account)
            .await
            .map_err(ServerFilterError::Engine)
    }

    pub(crate) async fn create(
        &self,
        account_id: &str,
        filter: ServerFilterCreate,
    ) -> Result<ServerFilterId, ServerFilterError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(ServerFilterError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .filter_create(&account, filter)
            .await
            .map_err(ServerFilterError::Engine)
    }

    pub(crate) async fn update(
        &self,
        account_id: &str,
        filter: ServerFilterId,
        patch: ServerFilterPatch,
    ) -> Result<(), ServerFilterError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(ServerFilterError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .filter_update(&account, filter, patch)
            .await
            .map_err(ServerFilterError::Engine)
    }

    pub(crate) async fn delete(
        &self,
        account_id: &str,
        filter: ServerFilterId,
    ) -> Result<(), ServerFilterError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(ServerFilterError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .filter_delete(&account, filter)
            .await
            .map_err(ServerFilterError::Engine)
    }

    pub(crate) async fn validate(
        &self,
        account_id: &str,
        filter: ServerFilterCreate,
    ) -> Result<FilterValidation, ServerFilterError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(ServerFilterError::Attach)?;
        let account = AccountId(account_id.to_string());
        action_account
            .engine
            .filter_validate(&account, filter)
            .await
            .map_err(ServerFilterError::Engine)
    }
}
