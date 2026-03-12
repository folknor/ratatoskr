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
    use super::find_header_value_case_insensitive;

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
}
