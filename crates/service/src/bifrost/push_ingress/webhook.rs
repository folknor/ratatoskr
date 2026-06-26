use std::sync::Arc;

use bifrost_types::{HintPayload, PushSource};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{PushIngress, RoutingKey};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphNotificationEnvelope {
    value: Vec<GraphNotification>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphNotification {
    resource: String,
    client_state: Option<String>,
}

pub(crate) async fn run_loopback_listener(ingress: Arc<PushIngress>, addr: String) {
    let listener = match TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("graph push ingress failed to bind {addr}: {error}");
            return;
        }
    };
    let cancel = ingress.cancel_token();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let ingress = Arc::clone(&ingress);
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(ingress, stream).await {
                                log::debug!("graph push ingress request dropped: {error}");
                            }
                        });
                    }
                    Err(error) => log::debug!("graph push ingress accept failed: {error}"),
                }
            }
        }
    }
}

async fn handle_connection(
    ingress: Arc<PushIngress>,
    mut stream: tokio::net::TcpStream,
) -> Result<(), String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|error| format!("read webhook request: {error}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    if let Some(token) = validation_token(&request) {
        write_response(&mut stream, "200 OK", "text/plain", token.as_bytes()).await?;
        return Ok(());
    }
    let Some(body_start) = request.find("\r\n\r\n").map(|idx| idx + 4) else {
        write_response(&mut stream, "400 Bad Request", "text/plain", b"bad request").await?;
        return Ok(());
    };
    let body = &buf[body_start..n];
    match handle_notification(&ingress, body).await {
        Ok(()) => write_response(&mut stream, "202 Accepted", "text/plain", b"accepted").await?,
        Err(error) => {
            log::debug!("graph push ingress invalid notification: {error}");
            write_response(&mut stream, "400 Bad Request", "text/plain", b"bad request").await?;
        }
    }
    Ok(())
}

fn validation_token(request: &str) -> Option<&str> {
    let request_line = request.lines().next()?;
    let path = request_line.split_whitespace().nth(1)?;
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "validationToken").then_some(value)
    })
}

pub(crate) async fn handle_notification(ingress: &PushIngress, body: &[u8]) -> Result<(), String> {
    let envelope: GraphNotificationEnvelope =
        serde_json::from_slice(body).map_err(|error| format!("parse Graph webhook: {error}"))?;
    for notification in envelope.value {
        let _ = notification.client_state.as_deref();
        let key = RoutingKey::GraphResource(notification.resource);
        if let Some(account_id) = ingress.route(&key).await {
            log::debug!("graph push ingress routed notification to {account_id}");
            ingress.on_validated(
                account_id,
                PushSource::GraphSubscription,
                HintPayload::Unknown,
            );
        } else {
            log::debug!("graph push ingress dropped unrouted notification");
        }
    }
    Ok(())
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|error| format!("write response headers: {error}"))?;
    stream
        .write_all(body)
        .await
        .map_err(|error| format!("write response body: {error}"))?;
    Ok(())
}
