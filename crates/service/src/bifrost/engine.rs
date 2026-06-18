use std::sync::Arc;

use bifrost_net::BandwidthMeter;
use bifrost_sync::{Error, SyncEngine};

use super::SqliteCheckpointStore;

#[derive(Clone)]
pub struct BifrostSyncEngine {
    engine: Arc<SyncEngine>,
    checkpoints: Arc<SqliteCheckpointStore>,
}

impl BifrostSyncEngine {
    pub fn build(
        checkpoints: SqliteCheckpointStore,
        bandwidth: Option<Arc<BandwidthMeter>>,
    ) -> Result<Self, Error> {
        let checkpoint_handle = Arc::new(checkpoints.clone());
        let mut builder = SyncEngine::builder().checkpoints(checkpoint_handle);
        if let Some(meter) = bandwidth {
            builder = builder.with_bandwidth_meter(meter);
        }
        Ok(Self {
            engine: Arc::new(builder.build()?),
            checkpoints: Arc::new(checkpoints),
        })
    }

    #[must_use]
    pub fn engine(&self) -> Arc<SyncEngine> {
        Arc::clone(&self.engine)
    }

    #[must_use]
    pub fn checkpoints(&self) -> Arc<SqliteCheckpointStore> {
        Arc::clone(&self.checkpoints)
    }
}
