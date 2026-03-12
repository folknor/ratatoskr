use std::path::PathBuf;
use std::sync::Arc;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::progress::ProgressReporter;
use crate::provider::crypto::AppCryptoState;
use crate::search::SearchState;

#[derive(Clone)]
pub struct ProviderStates {
    pub gmail: Arc<GmailState>,
    pub jmap: Arc<JmapState>,
    pub graph: Arc<GraphState>,
    encryption_key: [u8; 32],
}

impl ProviderStates {
    pub fn new(
        gmail: Arc<GmailState>,
        jmap: Arc<JmapState>,
        graph: Arc<GraphState>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            gmail,
            jmap,
            graph,
            encryption_key,
        }
    }

    pub fn encryption_key(&self) -> [u8; 32] {
        self.encryption_key
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: DbState,
    pub body_store: BodyStoreState,
    pub inline_images: InlineImageStoreState,
    pub search: SearchState,
    pub crypto: AppCryptoState,
    pub providers: ProviderStates,
    pub progress: Arc<dyn ProgressReporter>,
    pub app_data_dir: PathBuf,
}
