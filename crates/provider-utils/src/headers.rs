/// Inject a `Disposition-Notification-To` header into raw RFC 2822 bytes.
///
/// Extracts the sender address from the existing `From:` header and adds
/// `Disposition-Notification-To: <sender>` before the header/body separator.
/// If the header already exists or no `From:` address is found, returns the
/// bytes unchanged.
pub fn inject_read_receipt_header(raw: &[u8]) -> Vec<u8> {
    // Don't add if already present — only check the header block (before
    // the \r\n\r\n separator) to avoid false positives from body content.
    let raw_str = String::from_utf8_lossy(raw);
    let header_block = raw_str
        .split_once("\r\n\r\n")
        .map_or(raw_str.as_ref(), |(headers, _)| headers);
    if header_block
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("disposition-notification-to:"))
    {
        return raw.to_vec();
    }

    // Extract sender from From: header (header block only)
    let from_addr = header_block.lines().find_map(|line| {
        if !line.to_ascii_lowercase().starts_with("from:") {
            return None;
        }
        let value = line["from:".len()..].trim();
        // Extract email from "Name <email>" or bare "email" format
        if let Some(start) = value.rfind('<') {
            value[start + 1..].find('>').map(|end| &value[start + 1..start + 1 + end])
        } else {
            Some(value)
        }
    });

    let Some(sender) = from_addr else {
        return raw.to_vec();
    };

    // Find header/body separator and inject before it
    let separator = b"\r\n\r\n";
    let Some(pos) = raw
        .windows(separator.len())
        .position(|w| w == separator)
    else {
        return raw.to_vec();
    };

    let header_line = format!("Disposition-Notification-To: <{sender}>\r\n");
    let mut result = Vec::with_capacity(raw.len() + header_line.len());
    result.extend_from_slice(&raw[..pos]);
    result.extend_from_slice(b"\r\n");
    result.extend_from_slice(header_line.as_bytes());
    // The original separator was \r\n\r\n; we consumed up to `pos`, added \r\n + header + \r\n,
    // now we need the final \r\n to end the headers section.
    // We already wrote \r\n after the existing headers and the header line ends with \r\n,
    // so we just need one more \r\n for the blank line.
    result.extend_from_slice(&raw[pos + 2..]); // skip the first \r\n of separator, keep the second \r\n
    result
}

/// Inject a read-receipt header into a base64url-encoded RFC 2822 message.
///
/// Decodes the message, injects `Disposition-Notification-To`, and re-encodes.
pub fn inject_read_receipt_header_base64url(raw_base64url: &str) -> Result<String, String> {
    let raw_bytes = super::encoding::decode_base64url_nopad(raw_base64url)?;
    let patched = inject_read_receipt_header(&raw_bytes);
    Ok(super::encoding::encode_base64url_nopad(&patched))
}

pub fn find_header_value_case_insensitive<T, FName, FValue>(
    headers: &[T],
    name: &str,
    header_name: FName,
    header_value: FValue,
) -> Option<String>
where
    FName: Fn(&T) -> &str,
    FValue: Fn(&T) -> &str,
{
    headers
        .iter()
        .find(|header| header_name(header).eq_ignore_ascii_case(name))
        .map(|header| header_value(header).to_string())
}

#[cfg(test)]
mod tests {
    use super::{find_header_value_case_insensitive, inject_read_receipt_header};

    #[derive(Clone)]
    struct Header {
        name: &'static str,
        value: &'static str,
    }

    #[test]
    fn finds_case_insensitive_header() {
        let headers = vec![Header {
            name: "Message-ID",
            value: "<id@example.com>",
        }];
        let value =
            find_header_value_case_insensitive(&headers, "message-id", |h| h.name, |h| h.value);
        assert_eq!(value.as_deref(), Some("<id@example.com>"));
    }

    #[test]
    fn injects_read_receipt_header() {
        let raw = b"From: alice@example.com\r\nTo: bob@example.com\r\nSubject: Test\r\n\r\nBody";
        let result = inject_read_receipt_header(raw);
        let result_str = String::from_utf8(result).expect("valid utf8");
        assert!(result_str.contains("Disposition-Notification-To: <alice@example.com>"));
        assert!(result_str.contains("\r\n\r\nBody"));
    }

    #[test]
    fn injects_read_receipt_header_with_name() {
        let raw = b"From: \"Alice Smith\" <alice@example.com>\r\nTo: bob@example.com\r\n\r\nBody";
        let result = inject_read_receipt_header(raw);
        let result_str = String::from_utf8(result).expect("valid utf8");
        assert!(result_str.contains("Disposition-Notification-To: <alice@example.com>"));
    }

    #[test]
    fn does_not_duplicate_read_receipt_header() {
        let raw = b"From: alice@example.com\r\nDisposition-Notification-To: <alice@example.com>\r\n\r\nBody";
        let result = inject_read_receipt_header(raw);
        assert_eq!(result, raw.to_vec());
    }

    #[test]
    fn no_crash_without_from() {
        let raw = b"To: bob@example.com\r\nSubject: Test\r\n\r\nBody";
        let result = inject_read_receipt_header(raw);
        assert_eq!(result, raw.to_vec());
    }
}
