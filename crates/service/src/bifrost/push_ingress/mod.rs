pub mod pubsub;
pub mod webhook;

use std::collections::HashMap;
use std::sync::Arc;

use bifrost_types::{
    AccountId, HintPayload, InvalidationHint, InvalidationSink, PushSource, WatchEvent,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoutingKey {
    GmailEmail(String),
    GraphResource(String),
}

#[derive(Debug, Clone, Default)]
pub struct PushIngressConfig {
    pub gmail_pubsub_subscription: Option<String>,
    pub graph_loopback_addr: Option<String>,
    pub graph_notification_url: Option<String>,
    pub gmail_pubsub_topic: Option<String>,
    pub gmail_pubsub_mock_endpoint: Option<String>,
}

impl PushIngressConfig {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            gmail_pubsub_subscription: std::env::var("RATATOSKR_GMAIL_PUBSUB_SUBSCRIPTION").ok(),
            graph_loopback_addr: std::env::var("RATATOSKR_GRAPH_PUSH_LOOPBACK").ok(),
            graph_notification_url: std::env::var("RATATOSKR_GRAPH_PUSH_NOTIFICATION_URL").ok(),
            gmail_pubsub_topic: std::env::var("RATATOSKR_GMAIL_PUBSUB_TOPIC").ok(),
            gmail_pubsub_mock_endpoint: std::env::var("RATATOSKR_GMAIL_PUBSUB_MOCK_ENDPOINT").ok(),
        }
    }
}

pub struct PushIngress {
    sink: Arc<dyn InvalidationSink>,
    routing: Mutex<HashMap<RoutingKey, String>>,
    config: PushIngressConfig,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl PushIngress {
    #[must_use]
    pub fn new(sink: Arc<dyn InvalidationSink>, config: PushIngressConfig) -> Arc<Self> {
        Arc::new(Self {
            sink,
            routing: Mutex::new(HashMap::new()),
            config,
            tasks: Mutex::new(Vec::new()),
            cancel: CancellationToken::new(),
        })
    }

    pub async fn spawn(self: &Arc<Self>) {
        if let Some(addr) = self.config.graph_loopback_addr.clone() {
            let ingress = Arc::clone(self);
            self.tasks.lock().await.push(tokio::spawn(async move {
                webhook::run_loopback_listener(ingress, addr).await;
            }));
        }
        if self.config.gmail_pubsub_subscription.is_some()
            || self.config.gmail_pubsub_mock_endpoint.is_some()
        {
            let ingress = Arc::clone(self);
            self.tasks.lock().await.push(tokio::spawn(async move {
                pubsub::run_pull_subscriber(ingress).await;
            }));
        }
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let tasks = std::mem::take(&mut *self.tasks.lock().await);
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
    }

    pub async fn register(&self, key: RoutingKey, account_id: String) {
        self.routing.lock().await.insert(key, account_id);
    }

    pub async fn unregister_account(&self, account_id: &str) {
        self.routing
            .lock()
            .await
            .retain(|_, routed| routed != account_id);
    }

    pub async fn route(&self, key: &RoutingKey) -> Option<String> {
        let routing = self.routing.lock().await;
        if let Some(account_id) = routing.get(key) {
            return Some(account_id.clone());
        }
        // Only Graph resources fall back to prefix matching; any other key
        // type already missed the exact-match lookup above.
        let RoutingKey::GraphResource(resource) = key else {
            return None;
        };
        let resource = resource.trim_start_matches('/');
        routing.iter().find_map(|(candidate, account_id)| {
            let RoutingKey::GraphResource(prefix) = candidate else {
                return None;
            };
            let prefix = prefix.trim_start_matches('/');
            resource.starts_with(prefix).then(|| account_id.clone())
        })
    }

    pub fn on_validated(&self, account_id: String, source: PushSource, payload: HintPayload) {
        self.sink.push(
            AccountId(account_id),
            WatchEvent::Invalidated {
                hint: InvalidationHint { source, payload },
            },
        );
    }

    #[must_use]
    pub fn config(&self) -> &PushIngressConfig {
        &self.config
    }

    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use bifrost_types::{AccountId, WatchEvent};

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: StdMutex<Vec<(AccountId, WatchEvent)>>,
    }

    impl InvalidationSink for RecordingSink {
        fn push(&self, account: AccountId, event: WatchEvent) {
            self.events
                .lock()
                .expect("events mutex poisoned")
                .push((account, event));
        }
    }

    impl RecordingSink {
        fn len(&self) -> usize {
            self.events.lock().expect("events mutex poisoned").len()
        }

        fn last_account(&self) -> Option<String> {
            self.events
                .lock()
                .expect("events mutex poisoned")
                .last()
                .map(|(account, _)| account.0.clone())
        }
    }

    #[tokio::test]
    async fn gmail_pubsub_routes_valid_harness_envelope() {
        let sink = Arc::new(RecordingSink::default());
        let ingress = PushIngress::new(
            Arc::clone(&sink) as Arc<dyn InvalidationSink>,
            PushIngressConfig::default(),
        );
        ingress
            .register(
                RoutingKey::GmailEmail("user@example.com".to_string()),
                "acct-gmail".to_string(),
            )
            .await;

        super::pubsub::handle_harness_message(
            &ingress,
            br#"{"emailAddress":"user@example.com","historyId":"42"}"#,
        )
        .await
        .expect("valid Gmail Pub/Sub harness envelope");

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.last_account().as_deref(), Some("acct-gmail"));
    }

    #[tokio::test]
    async fn graph_webhook_routes_by_resource_prefix() {
        let sink = Arc::new(RecordingSink::default());
        let ingress = PushIngress::new(
            Arc::clone(&sink) as Arc<dyn InvalidationSink>,
            PushIngressConfig::default(),
        );
        ingress
            .register(
                RoutingKey::GraphResource("me/messages".to_string()),
                "acct-graph".to_string(),
            )
            .await;

        super::webhook::handle_notification(
            &ingress,
            br#"{"value":[{"resource":"me/messages/AAMkAGI2","clientState":"opaque"}]}"#,
        )
        .await
        .expect("valid Graph webhook envelope");

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.last_account().as_deref(), Some("acct-graph"));
    }

    #[tokio::test]
    async fn malformed_ingress_payloads_are_dropped() {
        let sink = Arc::new(RecordingSink::default());
        let ingress = PushIngress::new(
            Arc::clone(&sink) as Arc<dyn InvalidationSink>,
            PushIngressConfig::default(),
        );

        let gmail = super::pubsub::handle_harness_message(&ingress, b"not json").await;
        let graph = super::webhook::handle_notification(&ingress, b"not json").await;

        assert!(gmail.is_err());
        assert!(graph.is_err());
        assert_eq!(sink.len(), 0);
    }
}
