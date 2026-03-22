use serde::{Deserialize, Serialize};

use super::client::JmapClient;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A Sieve script as returned by our API layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SieveScript {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    /// The raw Sieve script text (RFC 5228). Only populated by [`get_sieve_script`].
    pub content: Option<String>,
}

/// Result of server-side Sieve script validation.
#[derive(Debug, Clone)]
pub struct SieveValidationResult {
    pub is_valid: bool,
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Capability check
// ---------------------------------------------------------------------------

/// Returns `true` if the connected JMAP server advertises Sieve support.
pub fn server_supports_sieve(client: &JmapClient) -> bool {
    let inner = client.inner();
    let session = inner.session();
    session.sieve_capabilities().is_some()
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// List all Sieve scripts on the server (without fetching script content).
pub async fn list_sieve_scripts(client: &JmapClient) -> Result<Vec<SieveScript>, String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    // Query all script IDs.
    let query_response = inner
        .sieve_script_query(
            None::<jmap_client::core::query::Filter<jmap_client::sieve::query::Filter>>,
            None::<Vec<jmap_client::core::query::Comparator<jmap_client::sieve::query::Comparator>>>,
        )
        .await
        .map_err(|e| format!("SieveScript/query: {e}"))?;

    let ids = query_response.ids();
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Batch-get metadata for all scripts.
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let mut get = jmap_client::sieve::SieveScriptGet::new(&account_id);
    get.ids(ids);
    get.properties([
        jmap_client::sieve::Property::Id,
        jmap_client::sieve::Property::Name,
        jmap_client::sieve::Property::IsActive,
    ]);
    let handle = request
        .call(get)
        .map_err(|e| format!("SieveScript/get: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("SieveScript/get: {e}"))?;

    let mut get_response = response
        .get(&handle)
        .map_err(|e| format!("SieveScript/get: {e}"))?;

    let scripts = get_response
        .take_list()
        .into_iter()
        .map(|s| SieveScript {
            id: s.id.unwrap_or_default(),
            name: s.name.unwrap_or_default(),
            is_active: s.is_active.unwrap_or(false),
            content: None,
        })
        .collect();

    Ok(scripts)
}

// ---------------------------------------------------------------------------
// Get (single, with content)
// ---------------------------------------------------------------------------

/// Fetch a single Sieve script including its content (downloaded from the blob store).
pub async fn get_sieve_script(
    client: &JmapClient,
    script_id: &str,
) -> Result<SieveScript, String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    // Get metadata + blob ID.
    let script = inner
        .sieve_script_get(
            script_id,
            Some([
                jmap_client::sieve::Property::Id,
                jmap_client::sieve::Property::Name,
                jmap_client::sieve::Property::IsActive,
                jmap_client::sieve::Property::BlobId,
            ]),
        )
        .await
        .map_err(|e| format!("SieveScript/get {script_id}: {e}"))?
        .ok_or_else(|| format!("Sieve script {script_id} not found"))?;

    let blob_id = script
        .blob_id
        .as_deref()
        .ok_or_else(|| format!("Sieve script {script_id} has no blob ID"))?;

    // Download the blob to get the script text.
    let blob_bytes = inner
        .download(blob_id)
        .await
        .map_err(|e| format!("Sieve blob download {blob_id}: {e}"))?;

    let content = String::from_utf8(blob_bytes)
        .map_err(|e| format!("Sieve script {script_id} content is not valid UTF-8: {e}"))?;

    Ok(SieveScript {
        id: script.id.unwrap_or_default(),
        name: script.name.unwrap_or_default(),
        is_active: script.is_active.unwrap_or(false),
        content: Some(content),
    })
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Create a new Sieve script on the server. Returns the server-assigned script ID.
pub async fn create_sieve_script(
    client: &JmapClient,
    name: &str,
    content: &str,
    activate: bool,
) -> Result<String, String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    let script = inner
        .sieve_script_create(name, content.as_bytes().to_vec(), activate)
        .await
        .map_err(|e| format!("SieveScript/create: {e}"))?;

    let id = script
        .id
        .ok_or_else(|| "SieveScript/create returned no ID".to_string())?;

    log::info!("Created Sieve script {id:?} (name={name:?}, active={activate})");
    Ok(id)
}

// ---------------------------------------------------------------------------
// Update (replace content)
// ---------------------------------------------------------------------------

/// Replace the content of an existing Sieve script.
///
/// If `activate` is `Some(true)`, the script is activated after update.
pub async fn update_sieve_script(
    client: &JmapClient,
    script_id: &str,
    content: &str,
    activate: Option<bool>,
) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    inner
        .sieve_script_replace(script_id, content.as_bytes().to_vec(), activate.unwrap_or(false))
        .await
        .map_err(|e| format!("SieveScript/replace {script_id}: {e}"))?;

    log::info!("Updated Sieve script {script_id:?}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

/// Rename an existing Sieve script.
///
/// If `activate` is `Some(true)`, the script is activated after renaming.
pub async fn rename_sieve_script(
    client: &JmapClient,
    script_id: &str,
    new_name: &str,
    activate: Option<bool>,
) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    inner
        .sieve_script_rename(script_id, new_name, activate.unwrap_or(false))
        .await
        .map_err(|e| format!("SieveScript/rename {script_id}: {e}"))?;

    log::info!("Renamed Sieve script {script_id:?} to {new_name:?}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Delete a Sieve script from the server.
pub async fn delete_sieve_script(
    client: &JmapClient,
    script_id: &str,
) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    inner
        .sieve_script_destroy(script_id)
        .await
        .map_err(|e| format!("SieveScript/destroy {script_id}: {e}"))?;

    log::info!("Deleted Sieve script {script_id:?}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Activate / Deactivate
// ---------------------------------------------------------------------------

/// Activate a Sieve script (deactivates any previously active script).
pub async fn activate_sieve_script(
    client: &JmapClient,
    script_id: &str,
) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    inner
        .sieve_script_activate(script_id)
        .await
        .map_err(|e| format!("SieveScript/activate {script_id}: {e}"))?;

    log::info!("Activated Sieve script {script_id:?}");
    Ok(())
}

/// Deactivate the currently active Sieve script.
pub async fn deactivate_sieve_script(client: &JmapClient) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    inner
        .sieve_script_deactivate()
        .await
        .map_err(|e| format!("SieveScript/deactivate: {e}"))?;

    log::info!("Deactivated active Sieve script");
    Ok(())
}

// ---------------------------------------------------------------------------
// Validate
// ---------------------------------------------------------------------------

/// Validate Sieve script content on the server without saving it.
pub async fn validate_sieve_script(
    client: &JmapClient,
    content: &str,
) -> Result<SieveValidationResult, String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    match inner
        .sieve_script_validate(content.as_bytes().to_vec())
        .await
    {
        Ok(()) => Ok(SieveValidationResult {
            is_valid: true,
            error_message: None,
        }),
        Err(e) => {
            // The validate method returns an Err with the validation error
            // from the server. We distinguish server-reported validation
            // failures from transport/protocol errors by checking if it looks
            // like a SetError (which is what SieveScriptValidateResponse wraps).
            let msg = e.to_string();
            if msg.contains("serverFail") || msg.contains("connection") {
                // Likely a transport error — propagate.
                Err(format!("SieveScript/validate: {msg}"))
            } else {
                // Validation failure reported by the server.
                Ok(SieveValidationResult {
                    is_valid: false,
                    error_message: Some(msg),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sieve_script_serialization_roundtrip() {
        let script = SieveScript {
            id: "s1".to_string(),
            name: "My Filter".to_string(),
            is_active: true,
            content: Some("require \"fileinto\";\nfileinto \"INBOX\";".to_string()),
        };

        let json = serde_json::to_string(&script).expect("serialize");
        let deserialized: SieveScript = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.id, "s1");
        assert_eq!(deserialized.name, "My Filter");
        assert!(deserialized.is_active);
        assert_eq!(
            deserialized.content.as_deref(),
            Some("require \"fileinto\";\nfileinto \"INBOX\";")
        );
    }

    #[test]
    fn sieve_script_without_content() {
        let script = SieveScript {
            id: "s2".to_string(),
            name: "Vacation".to_string(),
            is_active: false,
            content: None,
        };

        let json = serde_json::to_string(&script).expect("serialize");
        let deserialized: SieveScript = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.id, "s2");
        assert!(!deserialized.is_active);
        assert!(deserialized.content.is_none());
    }

    #[test]
    fn validation_result_valid() {
        let result = SieveValidationResult {
            is_valid: true,
            error_message: None,
        };
        assert!(result.is_valid);
        assert!(result.error_message.is_none());
    }

    #[test]
    fn validation_result_invalid() {
        let result = SieveValidationResult {
            is_valid: false,
            error_message: Some("line 3: unknown command \"foobar\"".to_string()),
        };
        assert!(!result.is_valid);
        assert!(result.error_message.unwrap().contains("foobar"));
    }
}
