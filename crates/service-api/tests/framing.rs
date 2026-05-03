use service_api::{
    BoundedLineReader, FrameError, JsonRpcRequest, ParsedClientMessage, RequestParams,
    parse_client_message, write_message,
};
use tokio::io::AsyncWriteExt;

#[test]
fn parses_health_ping_request_with_empty_object_params() -> Result<(), Box<dyn std::error::Error>> {
    let line = r#"{"jsonrpc":"2.0","id":42,"method":"health.ping","params":{}}"#;
    let parsed = parse_client_message(line)?;
    assert_eq!(
        parsed,
        ParsedClientMessage::Request {
            id: 42,
            params: RequestParams::HealthPing
        }
    );
    Ok(())
}

#[test]
fn parses_health_ping_request_with_null_params() -> Result<(), Box<dyn std::error::Error>> {
    let line = r#"{"jsonrpc":"2.0","id":42,"method":"health.ping","params":null}"#;
    let parsed = parse_client_message(line)?;
    assert_eq!(
        parsed,
        ParsedClientMessage::Request {
            id: 42,
            params: RequestParams::HealthPing
        }
    );
    Ok(())
}

#[tokio::test]
async fn writes_compact_single_line_messages() -> Result<(), Box<dyn std::error::Error>> {
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    write_message(
        &JsonRpcRequest::new(7, &RequestParams::HealthPing),
        &mut writer,
    )
    .await?;
    writer.shutdown().await?;

    let mut lines = BoundedLineReader::new(&mut reader, 1024);
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| std::io::Error::other("missing frame"))?;
    assert_eq!(
        line,
        r#"{"jsonrpc":"2.0","id":7,"method":"health.ping","params":null}"#
    );
    Ok(())
}

#[tokio::test]
async fn bounded_reader_rejects_oversize_while_reading(
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut writer, reader) = tokio::io::duplex(128);
    tokio::spawn(async move {
        let _ = writer.write_all(b"aaaaaaaaaaaaaaaaa\n").await;
        let _ = writer.write_all(b"{}\n").await;
    });

    let mut lines = BoundedLineReader::new(reader, 8);
    let first = lines.next_line().await;
    assert!(matches!(first, Err(FrameError::TooLarge)));
    let second = lines
        .next_line()
        .await?
        .ok_or_else(|| std::io::Error::other("missing second frame"))?;
    assert_eq!(second, "{}");
    Ok(())
}
