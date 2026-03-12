use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};

pub fn decode_base64_standard(input: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode: {e}"))
}

pub fn encode_base64_standard(data: &[u8]) -> String {
    STANDARD.encode(data)
}

pub fn decode_base64url_nopad(input: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|e| format!("base64url decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{decode_base64_standard, decode_base64url_nopad, encode_base64_standard};

    #[test]
    fn round_trips_standard_base64() {
        let encoded = encode_base64_standard(b"hello");
        let decoded = decode_base64_standard(&encoded).expect("decode should succeed");
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn decodes_base64url_without_padding() {
        let decoded = decode_base64url_nopad("SGVsbG8").expect("decode should succeed");
        assert_eq!(decoded, b"Hello");
    }
}
