use quick_xml::Reader;
use quick_xml::events::Event;

const AUTODISCOVER_URL: &str =
    "https://outlook.office365.com/autodiscover/autodiscover.xml";

/// A shared/delegate mailbox discovered via Exchange Autodiscover.
#[derive(Debug, Clone)]
pub struct SharedMailbox {
    pub smtp_address: String,
    pub display_name: Option<String>,
    /// E.g. "Delegate", "TeamMailbox", etc.
    pub mailbox_type: String,
}

/// Discover shared/delegate mailboxes for a user via Exchange Autodiscover XML.
///
/// Calls the Autodiscover endpoint with an OAuth bearer token and parses
/// `AlternativeMailbox` elements from the response.
pub async fn discover_shared_mailboxes(
    http_client: &reqwest::Client,
    access_token: &str,
    user_email: &str,
) -> Result<Vec<SharedMailbox>, String> {
    let request_body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006">
  <Request>
    <EMailAddress>{user_email}</EMailAddress>
    <AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema>
  </Request>
</Autodiscover>"#
    );

    let resp = http_client
        .post(AUTODISCOVER_URL)
        .header("Content-Type", "text/xml")
        .header("Authorization", format!("Bearer {access_token}"))
        .body(request_body)
        .send()
        .await
        .map_err(|e| format!("Autodiscover request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Autodiscover returned {status}: {body}"
        ));
    }

    let xml = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Autodiscover response: {e}"))?;

    Ok(parse_alternative_mailboxes(&xml))
}

/// Parse `AlternativeMailbox` elements from Autodiscover XML response.
fn parse_alternative_mailboxes(xml: &str) -> Vec<SharedMailbox> {
    let mut reader = Reader::from_str(xml);
    let mut mailboxes = Vec::new();

    let mut in_alternative_mailbox = false;
    let mut current_type = String::new();
    let mut current_display_name = String::new();
    let mut current_smtp = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "AlternativeMailbox" {
                    in_alternative_mailbox = true;
                    current_type.clear();
                    current_display_name.clear();
                    current_smtp.clear();
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_alternative_mailbox {
                    let trimmed = buf.trim();
                    match current_tag.as_str() {
                        "Type" => current_type = trimmed.to_string(),
                        "DisplayName" => current_display_name = trimmed.to_string(),
                        "SmtpAddress" => current_smtp = trimmed.to_string(),
                        _ => {}
                    }
                }
                if name == "AlternativeMailbox" {
                    in_alternative_mailbox = false;
                    if !current_smtp.is_empty() {
                        mailboxes.push(SharedMailbox {
                            smtp_address: current_smtp.clone(),
                            display_name: if current_display_name.is_empty() {
                                None
                            } else {
                                Some(current_display_name.clone())
                            },
                            mailbox_type: current_type.clone(),
                        });
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    mailboxes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_alternative_mailbox() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a">
  <Response>
    <Account>
      <AlternativeMailbox>
        <Type>Delegate</Type>
        <DisplayName>Sales Team</DisplayName>
        <SmtpAddress>sales@contoso.com</SmtpAddress>
      </AlternativeMailbox>
    </Account>
  </Response>
</Autodiscover>"#;

        let result = parse_alternative_mailboxes(xml);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].smtp_address, "sales@contoso.com");
        assert_eq!(result[0].display_name.as_deref(), Some("Sales Team"));
        assert_eq!(result[0].mailbox_type, "Delegate");
    }

    #[test]
    fn parse_multiple_alternative_mailboxes() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover>
  <Response>
    <Account>
      <AlternativeMailbox>
        <Type>Delegate</Type>
        <DisplayName>Sales Team</DisplayName>
        <SmtpAddress>sales@contoso.com</SmtpAddress>
      </AlternativeMailbox>
      <AlternativeMailbox>
        <Type>TeamMailbox</Type>
        <DisplayName>Engineering</DisplayName>
        <SmtpAddress>eng@contoso.com</SmtpAddress>
      </AlternativeMailbox>
      <AlternativeMailbox>
        <Type>Delegate</Type>
        <SmtpAddress>noreply@contoso.com</SmtpAddress>
      </AlternativeMailbox>
    </Account>
  </Response>
</Autodiscover>"#;

        let result = parse_alternative_mailboxes(xml);
        assert_eq!(result.len(), 3);

        assert_eq!(result[0].smtp_address, "sales@contoso.com");
        assert_eq!(result[0].mailbox_type, "Delegate");

        assert_eq!(result[1].smtp_address, "eng@contoso.com");
        assert_eq!(result[1].display_name.as_deref(), Some("Engineering"));
        assert_eq!(result[1].mailbox_type, "TeamMailbox");

        assert_eq!(result[2].smtp_address, "noreply@contoso.com");
        assert_eq!(result[2].display_name, None);
    }

    #[test]
    fn parse_empty_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover>
  <Response>
    <Account>
    </Account>
  </Response>
</Autodiscover>"#;

        let result = parse_alternative_mailboxes(xml);
        assert!(result.is_empty());
    }

    #[test]
    fn skip_mailbox_without_smtp_address() {
        let xml = r#"<Autodiscover>
  <Response>
    <Account>
      <AlternativeMailbox>
        <Type>Delegate</Type>
        <DisplayName>Broken Entry</DisplayName>
      </AlternativeMailbox>
    </Account>
  </Response>
</Autodiscover>"#;

        let result = parse_alternative_mailboxes(xml);
        assert!(result.is_empty());
    }
}
