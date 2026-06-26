use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bifrost_types::{HintPayload, PushSource};
use serde::Deserialize;

use super::{PushIngress, RoutingKey};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailPubsubEnvelope {
    email_address: String,
    history_id: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PubsubPushEnvelope {
    message: PubsubMessage,
}

#[derive(Debug, Deserialize)]
struct PubsubMessage {
    data: String,
}

pub(crate) async fn run_pull_subscriber(ingress: Arc<PushIngress>) {
    if let Some(endpoint) = ingress.config().gmail_pubsub_mock_endpoint.clone() {
        run_harness_mock_subscriber(ingress, endpoint).await;
        return;
    }
    let cancel = ingress.cancel_token();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                log::debug!("gmail pubsub ingress is configured but no production pull adapter is installed");
            }
        }
    }
}

async fn run_harness_mock_subscriber(ingress: Arc<PushIngress>, endpoint: String) {
    let cancel = ingress.cancel_token();
    let http = reqwest::Client::new();
    let url = format!(
        "{}/test/gmail/pubsub/messages",
        endpoint.trim_end_matches('/')
    );
    let mut seen = HashSet::new();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(100)) => {
                match http.get(&url).send().await {
                    Ok(response) => match response.json::<Vec<serde_json::Value>>().await {
                        Ok(messages) => {
                            if messages.is_empty() {
                                seen.clear();
                            }
                            for message in &messages {
                                let message_id = message
                                    .pointer("/message/messageId")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                if !seen.insert(message_id) {
                                    continue;
                                }
                                match serde_json::to_vec(message) {
                                    Ok(body) => {
                                        if let Err(error) = handle_pubsub_push_envelope(&ingress, &body).await {
                                            log::debug!("gmail mock pubsub message dropped: {error}");
                                        }
                                    }
                                    Err(error) => {
                                        log::debug!("gmail mock pubsub message encode failed: {error}");
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            log::debug!("gmail mock pubsub response parse failed: {error}");
                        }
                    },
                    Err(error) => {
                        log::debug!("gmail mock pubsub poll failed: {error}");
                    }
                }
            }
        }
    }
}

async fn handle_pubsub_push_envelope(ingress: &PushIngress, body: &[u8]) -> Result<(), String> {
    let envelope: PubsubPushEnvelope = serde_json::from_slice(body)
        .map_err(|error| format!("parse Pub/Sub push envelope: {error}"))?;
    let decoded = STANDARD
        .decode(envelope.message.data.as_bytes())
        .map_err(|error| format!("decode Pub/Sub data: {error}"))?;
    handle_harness_message(ingress, &decoded).await
}

pub(crate) async fn handle_harness_message(
    ingress: &PushIngress,
    body: &[u8],
) -> Result<(), String> {
    let envelope: GmailPubsubEnvelope = serde_json::from_slice(body)
        .map_err(|error| format!("parse Gmail Pub/Sub push: {error}"))?;
    let _ = envelope.history_id.as_ref();
    let key = RoutingKey::GmailEmail(envelope.email_address);
    let Some(account_id) = ingress.route(&key).await else {
        log::debug!("gmail pubsub ingress dropped unrouted notification");
        return Ok(());
    };
    log::debug!("gmail pubsub ingress routed notification to {account_id}");
    ingress.on_validated(account_id, PushSource::GmailPubsub, HintPayload::Unknown);
    Ok(())
}
