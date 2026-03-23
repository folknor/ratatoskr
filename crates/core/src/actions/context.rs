use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;

/// Dependencies needed by the action service.
///
/// Constructed once at app startup from pre-initialized stores.
/// All fields are cheaply cloneable (`Arc<Mutex<…>>` internally).
/// The service constructs `ProviderCtx` from these fields per-call —
/// callers never see `ProviderCtx`.
#[derive(Clone)]
pub struct ActionContext {
    pub db: DbState,
    pub body_store: BodyStoreState,
    pub inline_images: InlineImageStoreState,
    pub search: SearchState,
    pub encryption_key: [u8; 32],
}
