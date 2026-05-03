use crate::error::JsonRpcErrorObject;
use crate::notification::Notification;
use crate::request::RequestParams;
use crate::version::MAX_FRAME_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame too large")]
    TooLarge,
    #[error("invalid utf-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

pub struct BoundedLineReader<R> {
    reader: BufReader<R>,
    max_len: usize,
    buffer: Vec<u8>,
    discarding: bool,
}

impl<R: AsyncRead + Unpin> BoundedLineReader<R> {
    pub fn new(reader: R, max_len: usize) -> Self {
        Self {
            reader: BufReader::new(reader),
            max_len,
            buffer: Vec::new(),
            discarding: false,
        }
    }

    pub async fn next_line(&mut self) -> Result<Option<String>, FrameError> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.discarding {
                    self.discarding = false;
                    return Ok(None);
                }
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.buffer);
                return String::from_utf8(line).map(Some).map_err(FrameError::from);
            }

            if self.discarding {
                let consumed = match available.iter().position(|byte| *byte == b'\n') {
                    Some(pos) => {
                        self.discarding = false;
                        pos + 1
                    }
                    None => available.len(),
                };
                self.reader.consume(consumed);
                continue;
            }

            if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
                if self.buffer.len() + pos > self.max_len {
                    self.buffer.clear();
                    self.reader.consume(pos + 1);
                    return Err(FrameError::TooLarge);
                }
                self.buffer.extend_from_slice(&available[..pos]);
                self.reader.consume(pos + 1);
                let line = std::mem::take(&mut self.buffer);
                return String::from_utf8(line).map(Some).map_err(FrameError::from);
            }

            if self.buffer.len() + available.len() > self.max_len {
                self.buffer.clear();
                let consumed = available.len();
                self.reader.consume(consumed);
                self.discarding = true;
                return Err(FrameError::TooLarge);
            }

            self.buffer.extend_from_slice(available);
            let consumed = available.len();
            self.reader.consume(consumed);
        }
    }

    /// Test-only accessor for the in-flight buffer size, used to verify
    /// that the reader caps its allocation under a no-newline payload.
    #[cfg(test)]
    fn buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'static str,
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, params: &RequestParams) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: params.method_name(),
            params: params.params_value(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcSuccessResponse<T> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub result: T,
}

impl<T> JsonRpcSuccessResponse<T> {
    pub fn new(id: u64, result: T) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: &'static str,
    pub id: Option<u64>,
    pub error: JsonRpcErrorObject,
}

impl JsonRpcErrorResponse {
    pub fn new(id: Option<u64>, error: JsonRpcErrorObject) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            error,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawMessage {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<JsonRpcErrorObject>,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestParseError {
    #[error("malformed json: {0}")]
    MalformedJson(#[from] serde_json::Error),
    #[error("invalid request: {message}")]
    InvalidRequest {
        id: Option<u64>,
        message: String,
    },
}

impl RequestParseError {
    /// Returns the request id if it was extractable from the wire payload,
    /// even if validation later failed. Lets the dispatch loop respond with
    /// a correctly-correlated error instead of `id=null`.
    pub fn extracted_id(&self) -> Option<u64> {
        match self {
            Self::InvalidRequest { id, .. } => *id,
            Self::MalformedJson(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedClientMessage {
    Request { id: u64, params: RequestParams },
}

#[derive(Debug, Clone)]
pub enum ServiceResponse {
    Success(Value),
    Error(JsonRpcErrorObject),
}

#[derive(Debug, Clone)]
pub enum ParsedServiceMessage {
    Response {
        id: Option<u64>,
        response: ServiceResponse,
    },
    Notification(Notification),
}

pub fn parse_client_message(line: &str) -> Result<ParsedClientMessage, RequestParseError> {
    let raw: RawMessage = serde_json::from_str(line)?;
    let id_opt = raw.id.as_ref().and_then(extract_u64);

    if raw.jsonrpc.as_deref() != Some("2.0") {
        return Err(RequestParseError::InvalidRequest {
            id: id_opt,
            message: "jsonrpc must be 2.0".to_string(),
        });
    }
    let id = id_opt.ok_or_else(|| RequestParseError::InvalidRequest {
        id: None,
        message: "id must be a u64".to_string(),
    })?;
    let method = raw
        .method
        .as_deref()
        .ok_or_else(|| RequestParseError::InvalidRequest {
            id: Some(id),
            message: "missing method".to_string(),
        })?;
    if raw.result.is_some() || raw.error.is_some() {
        return Err(RequestParseError::InvalidRequest {
            id: Some(id),
            message: "request cannot contain result or error".to_string(),
        });
    }
    let params = RequestParams::from_method_params(method, raw.params).map_err(|message| {
        RequestParseError::InvalidRequest {
            id: Some(id),
            message,
        }
    })?;
    Ok(ParsedClientMessage::Request { id, params })
}

fn extract_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

pub fn parse_service_message(line: &str) -> Result<ParsedServiceMessage, RequestParseError> {
    let raw: RawMessage = serde_json::from_str(line)?;
    require_jsonrpc(&raw)?;
    // serde's Option<Value> deserialization collapses JSON `null` to `None`,
    // so `{"result": null}` and "no result field" are indistinguishable on
    // this side of the wire. Drive the response/notification choice off the
    // (id, method) shape rather than result/error presence; treat absent
    // result+error as Success(Value::Null).
    match (raw.id.as_ref(), raw.method.as_deref()) {
        (Some(_), None) => {
            let id = parse_response_id(raw.id.as_ref())?;
            let response = if let Some(error) = raw.error {
                ServiceResponse::Error(error)
            } else {
                ServiceResponse::Success(raw.result.unwrap_or(Value::Null))
            };
            Ok(ParsedServiceMessage::Response { id, response })
        }
        (None, None) if raw.error.is_some() => {
            // Parse-error response from the Service: id is null/absent,
            // error is present. Surface it with id=None.
            let error = raw
                .error
                .expect("guarded by the arm condition");
            Ok(ParsedServiceMessage::Response {
                id: None,
                response: ServiceResponse::Error(error),
            })
        }
        (None, Some(method)) => {
            let value = serde_json::json!({
                "method": method,
                "params": raw.params.unwrap_or(Value::Null),
            });
            let notification = serde_json::from_value(value)?;
            Ok(ParsedServiceMessage::Notification(notification))
        }
        _ => Err(RequestParseError::InvalidRequest {
            id: None,
            message: "message is neither response nor notification".to_string(),
        }),
    }
}

fn require_jsonrpc(raw: &RawMessage) -> Result<(), RequestParseError> {
    match raw.jsonrpc.as_deref() {
        Some("2.0") => Ok(()),
        _ => Err(RequestParseError::InvalidRequest {
            id: None,
            message: "jsonrpc must be 2.0".to_string(),
        }),
    }
}

fn parse_response_id(id: Option<&Value>) -> Result<Option<u64>, RequestParseError> {
    match id {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match extract_u64(value) {
            Some(id) => Ok(Some(id)),
            None => Err(RequestParseError::InvalidRequest {
                id: None,
                message: "response id must be a u64".to_string(),
            }),
        },
    }
}

pub async fn write_message<T, W>(value: &T, writer: &mut W) -> io::Result<()>
where
    T: Serialize,
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "serialized frame exceeds maximum size",
        ));
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub fn encode_message<T>(value: &T) -> io::Result<Vec<u8>>
where
    T: Serialize,
{
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "serialized frame exceeds maximum size",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// Feed a 1 MiB no-newline payload in 8 KiB chunks while sampling the
    /// reader's internal buffer. The cap is 64 KiB; the buffer must never
    /// exceed it. This is the "during read" requirement: a pathological
    /// producer cannot make the reader allocate past the cap before the
    /// `TooLarge` error fires.
    #[tokio::test]
    async fn bounded_reader_buffer_stays_capped_under_no_newline_payload() {
        let max_len = 64 * 1024;
        let chunk = vec![b'a'; 8 * 1024];
        let total_chunks = 128;

        let (mut writer, reader) = tokio::io::duplex(16 * 1024);
        let producer = tokio::spawn(async move {
            for _ in 0..total_chunks {
                if writer.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            let _ = writer.shutdown().await;
        });

        let mut lines = BoundedLineReader::new(reader, max_len);
        let mut peak = 0usize;
        let mut got_too_large = false;
        for _ in 0..(total_chunks * 4) {
            // Drive the reader forward in small steps. The duplex's 16 KiB
            // capacity backpressures the producer, so we never have more
            // than a few chunks in flight, but the cumulative payload is
            // 1 MiB - far past the 64 KiB cap.
            let next = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                lines.next_line(),
            )
            .await;
            peak = peak.max(lines.buffer_len());
            match next {
                Ok(Err(FrameError::TooLarge)) => {
                    got_too_large = true;
                    break;
                }
                Ok(Ok(_)) | Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        producer.abort();

        assert!(got_too_large, "expected TooLarge before exhausting payload");
        assert!(
            peak <= max_len,
            "buffer grew past cap: peak={peak}, max_len={max_len}"
        );
    }
}
