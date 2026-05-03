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
    #[error("invalid request: {0}")]
    InvalidRequest(String),
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
    require_jsonrpc(&raw)?;
    let id = parse_id(raw.id.as_ref())?;
    let method = raw
        .method
        .as_deref()
        .ok_or_else(|| RequestParseError::InvalidRequest("missing method".to_string()))?;
    if raw.result.is_some() || raw.error.is_some() {
        return Err(RequestParseError::InvalidRequest(
            "request cannot contain result or error".to_string(),
        ));
    }
    let params = RequestParams::from_method_params(method, raw.params)
        .map_err(RequestParseError::InvalidRequest)?;
    Ok(ParsedClientMessage::Request { id, params })
}

pub fn parse_service_message(line: &str) -> Result<ParsedServiceMessage, RequestParseError> {
    let raw: RawMessage = serde_json::from_str(line)?;
    require_jsonrpc(&raw)?;
    match (raw.id.as_ref(), raw.method.as_deref()) {
        (_, None) if raw.result.is_some() || raw.error.is_some() => {
            let id = parse_response_id(raw.id.as_ref())?;
            let response = match (raw.result, raw.error) {
                (Some(result), None) => ServiceResponse::Success(result),
                (None, Some(error)) => ServiceResponse::Error(error),
                _ => {
                    return Err(RequestParseError::InvalidRequest(
                        "response must contain result or error".to_string(),
                    ));
                }
            };
            Ok(ParsedServiceMessage::Response { id, response })
        }
        (None, Some(method)) => {
            let value = serde_json::json!({
                "method": method,
                "params": raw.params.unwrap_or(Value::Null),
            });
            let notification = serde_json::from_value(value)?;
            Ok(ParsedServiceMessage::Notification(notification))
        }
        _ => Err(RequestParseError::InvalidRequest(
            "message is neither response nor notification".to_string(),
        )),
    }
}

fn require_jsonrpc(raw: &RawMessage) -> Result<(), RequestParseError> {
    match raw.jsonrpc.as_deref() {
        Some("2.0") => Ok(()),
        _ => Err(RequestParseError::InvalidRequest(
            "jsonrpc must be 2.0".to_string(),
        )),
    }
}

fn parse_id(id: Option<&Value>) -> Result<u64, RequestParseError> {
    match id {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| RequestParseError::InvalidRequest("id must be u64".to_string())),
        _ => Err(RequestParseError::InvalidRequest(
            "id must be numeric".to_string(),
        )),
    }
}

fn parse_response_id(id: Option<&Value>) -> Result<Option<u64>, RequestParseError> {
    match id {
        None | Some(Value::Null) => Ok(None),
        Some(_) => parse_id(id).map(Some),
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
