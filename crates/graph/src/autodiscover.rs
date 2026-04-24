use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use super::ews::EwsHeaders;

const AUTODISCOVER_URL: &str = "https://outlook.office365.com/autodiscover/autodiscover.xml";

const AUTODISCOVER_SOAP_URL: &str = "https://outlook.office365.com/autodiscover/autodiscover.svc";

/// A shared/delegate mailbox discovered via Exchange Autodiscover.
#[derive(Debug, Clone)]
pub struct SharedMailbox {
    pub smtp_address: String,
    pub display_name: Option<String>,
    /// E.g. "Delegate", "TeamMailbox", etc.
    pub mailbox_type: String,
}

/// Routing information for public folder access via EWS.
#[derive(Debug, Clone)]
pub struct PublicFolderRouting {
    /// The hierarchy mailbox SMTP address (from PublicFolderInformation).
    pub hierarchy_mailbox: String,
    /// The mailbox server for X-PublicFolderMailbox (from InternalRpcClientServer).
    pub hierarchy_server: Option<String>,
}

impl PublicFolderRouting {
    /// Build EwsHeaders for hierarchy operations (browsing folders).
    pub fn hierarchy_headers(&self) -> EwsHeaders {
        EwsHeaders {
            anchor_mailbox: Some(self.hierarchy_mailbox.clone()),
            public_folder_mailbox: self.hierarchy_server.clone(),
        }
    }
}

/// Build EwsHeaders for content operations on a specific public folder.
pub fn build_content_headers(content_mailbox: &str) -> EwsHeaders {
    EwsHeaders {
        anchor_mailbox: Some(content_mailbox.to_string()),
        public_folder_mailbox: Some(content_mailbox.to_string()),
    }
}

/// Construct a synthetic SMTP address from a PR_REPLICA_LIST GUID and domain.
pub fn construct_replica_smtp(guid: &str, domain: &str) -> String {
    format!("{guid}@{domain}")
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
    let escaped_email = quick_xml::escape::escape(user_email);
    let request_body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006">
  <Request>
    <EMailAddress>{escaped_email}</EMailAddress>
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
        return Err(format!("Autodiscover returned {status}: {body}"));
    }

    let xml = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Autodiscover response: {e}"))?;

    Ok(parse_alternative_mailboxes(&xml))
}

// ── SOAP-based Autodiscover (public folder routing) ──────────

/// Build a GetUserSettings SOAP request body.
fn build_get_user_settings_soap(email: &str, settings: &[&str]) -> String {
    let escaped_email = quick_xml::escape::escape(email);
    let settings_xml: String = settings
        .iter()
        .map(|s| format!("          <a:Setting>{s}</a:Setting>"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:a="http://schemas.microsoft.com/exchange/2010/Autodiscover">
  <soap:Header>
    <a:RequestedServerVersion>Exchange2016</a:RequestedServerVersion>
  </soap:Header>
  <soap:Body>
    <a:GetUserSettingsRequestMessage>
      <a:Request>
        <a:Users>
          <a:User>
            <a:Mailbox>{escaped_email}</a:Mailbox>
          </a:User>
        </a:Users>
        <a:RequestedSettings>
{settings_xml}
        </a:RequestedSettings>
      </a:Request>
    </a:GetUserSettingsRequestMessage>
  </soap:Body>
</soap:Envelope>"#
    )
}

/// Send a GetUserSettings SOAP request and parse the response into name/value pairs.
async fn soap_get_user_settings(
    http_client: &reqwest::Client,
    access_token: &str,
    email: &str,
    settings: &[&str],
) -> Result<Vec<(String, String)>, String> {
    let body = build_get_user_settings_soap(email, settings);

    let resp = http_client
        .post(AUTODISCOVER_SOAP_URL)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("Authorization", format!("Bearer {access_token}"))
        .header(
            "SOAPAction",
            "\"http://schemas.microsoft.com/exchange/2010/Autodiscover/Autodiscover/GetUserSettings\"",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Autodiscover SOAP request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Autodiscover SOAP returned {status}: {body}"));
    }

    let xml = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Autodiscover SOAP response: {e}"))?;

    Ok(parse_user_settings(&xml))
}

/// Discover public folder routing information for a user.
///
/// Makes an Autodiscover SOAP `GetUserSettings` request to retrieve:
/// - `PublicFolderInformation` - the hierarchy mailbox SMTP address
/// - `InternalRpcClientServer` - the mailbox server (used as X-PublicFolderMailbox)
pub async fn discover_public_folder_routing(
    http_client: &reqwest::Client,
    access_token: &str,
    user_email: &str,
) -> Result<PublicFolderRouting, String> {
    log::info!("Discovering public folder routing for {user_email}");

    let settings = soap_get_user_settings(
        http_client,
        access_token,
        user_email,
        &["PublicFolderInformation", "InternalRpcClientServer"],
    )
    .await?;

    let mut hierarchy_mailbox: Option<String> = None;
    let mut hierarchy_server: Option<String> = None;

    for (name, value) in &settings {
        match name.as_str() {
            "PublicFolderInformation" => hierarchy_mailbox = Some(value.clone()),
            "InternalRpcClientServer" => hierarchy_server = Some(value.clone()),
            _ => {}
        }
    }

    let hierarchy_mailbox = hierarchy_mailbox
        .ok_or_else(|| "PublicFolderInformation not found in autodiscover response".to_string())?;

    log::info!(
        "Public folder routing: hierarchy_mailbox={hierarchy_mailbox}, server={hierarchy_server:?}"
    );

    Ok(PublicFolderRouting {
        hierarchy_mailbox,
        hierarchy_server,
    })
}

/// Discover the content mailbox for a public folder replica.
///
/// Given a synthetic SMTP address (e.g., `{GUID}@domain.com` from PR_REPLICA_LIST),
/// calls autodiscover to resolve its `AutoDiscoverSMTPAddress`, which is the actual
/// content mailbox routing address.
pub async fn discover_content_mailbox(
    http_client: &reqwest::Client,
    access_token: &str,
    replica_smtp: &str,
) -> Result<String, String> {
    log::info!("Discovering content mailbox for replica {replica_smtp}");

    let settings = soap_get_user_settings(
        http_client,
        access_token,
        replica_smtp,
        &["AutoDiscoverSMTPAddress"],
    )
    .await?;

    for (name, value) in &settings {
        if name == "AutoDiscoverSMTPAddress" {
            log::info!("Content mailbox for {replica_smtp}: {value}");
            return Ok(value.clone());
        }
    }

    Err(format!(
        "AutoDiscoverSMTPAddress not found for {replica_smtp}"
    ))
}

/// Parse `UserSetting` elements from a GetUserSettings SOAP response.
///
/// Extracts `<Name>` / `<Value>` pairs from `<UserSetting>` elements.
fn parse_user_settings(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut settings = Vec::new();

    let mut in_user_setting = false;
    let mut current_name = String::new();
    let mut current_value = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).to_string();
                if name == "UserSetting" {
                    in_user_setting = true;
                    current_name.clear();
                    current_value.clear();
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).to_string();
                if in_user_setting {
                    let trimmed = buf.trim();
                    match current_tag.as_str() {
                        "Name" => current_name = trimmed.to_string(),
                        "Value" => current_value = trimmed.to_string(),
                        _ => {}
                    }
                }
                if name == "UserSetting" {
                    in_user_setting = false;
                    if !current_name.is_empty() && !current_value.is_empty() {
                        settings.push((current_name.clone(), current_value.clone()));
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    settings
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
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).to_string();
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
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).to_string();
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
    fn parse_public_folder_routing_settings() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            xmlns:a="http://schemas.microsoft.com/exchange/2010/Autodiscover">
  <s:Body>
    <a:GetUserSettingsResponseMessage>
      <a:Response>
        <a:UserResponses>
          <a:UserResponse>
            <a:UserSettings>
              <a:UserSetting>
                <a:Name>PublicFolderInformation</a:Name>
                <a:Value>publicfolders@contoso.com</a:Value>
              </a:UserSetting>
              <a:UserSetting>
                <a:Name>InternalRpcClientServer</a:Name>
                <a:Value>server01.contoso.com</a:Value>
              </a:UserSetting>
            </a:UserSettings>
          </a:UserResponse>
        </a:UserResponses>
      </a:Response>
    </a:GetUserSettingsResponseMessage>
  </s:Body>
</s:Envelope>"#;

        let settings = parse_user_settings(xml);
        assert_eq!(settings.len(), 2);
        assert_eq!(settings[0].0, "PublicFolderInformation");
        assert_eq!(settings[0].1, "publicfolders@contoso.com");
        assert_eq!(settings[1].0, "InternalRpcClientServer");
        assert_eq!(settings[1].1, "server01.contoso.com");
    }

    #[test]
    fn parse_autodiscover_smtp_address() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            xmlns:a="http://schemas.microsoft.com/exchange/2010/Autodiscover">
  <s:Body>
    <a:GetUserSettingsResponseMessage>
      <a:Response>
        <a:UserResponses>
          <a:UserResponse>
            <a:UserSettings>
              <a:UserSetting>
                <a:Name>AutoDiscoverSMTPAddress</a:Name>
                <a:Value>contentmailbox@contoso.com</a:Value>
              </a:UserSetting>
            </a:UserSettings>
          </a:UserResponse>
        </a:UserResponses>
      </a:Response>
    </a:GetUserSettingsResponseMessage>
  </s:Body>
</s:Envelope>"#;

        let settings = parse_user_settings(xml);
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].0, "AutoDiscoverSMTPAddress");
        assert_eq!(settings[0].1, "contentmailbox@contoso.com");
    }

    #[test]
    fn parse_user_settings_empty_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetUserSettingsResponseMessage>
      <Response>
        <UserResponses>
          <UserResponse>
            <UserSettings />
          </UserResponse>
        </UserResponses>
      </Response>
    </GetUserSettingsResponseMessage>
  </s:Body>
</s:Envelope>"#;

        let settings = parse_user_settings(xml);
        assert!(settings.is_empty());
    }

    #[test]
    fn construct_replica_smtp_helper() {
        let smtp = construct_replica_smtp("1A2B3C4D-5E6F-7A8B-9C0D-1E2F3A4B5C6D", "contoso.com");
        assert_eq!(smtp, "1A2B3C4D-5E6F-7A8B-9C0D-1E2F3A4B5C6D@contoso.com");
    }

    #[test]
    fn hierarchy_headers_construction() {
        let routing = PublicFolderRouting {
            hierarchy_mailbox: "publicfolders@contoso.com".to_string(),
            hierarchy_server: Some("server01.contoso.com".to_string()),
        };
        let headers = routing.hierarchy_headers();
        assert_eq!(
            headers.anchor_mailbox.as_deref(),
            Some("publicfolders@contoso.com")
        );
        assert_eq!(
            headers.public_folder_mailbox.as_deref(),
            Some("server01.contoso.com")
        );
    }

    #[test]
    fn hierarchy_headers_without_server() {
        let routing = PublicFolderRouting {
            hierarchy_mailbox: "publicfolders@contoso.com".to_string(),
            hierarchy_server: None,
        };
        let headers = routing.hierarchy_headers();
        assert_eq!(
            headers.anchor_mailbox.as_deref(),
            Some("publicfolders@contoso.com")
        );
        assert_eq!(headers.public_folder_mailbox, None);
    }

    #[test]
    fn build_content_headers_construction() {
        let headers = build_content_headers("contentmailbox@contoso.com");
        assert_eq!(
            headers.anchor_mailbox.as_deref(),
            Some("contentmailbox@contoso.com")
        );
        assert_eq!(
            headers.public_folder_mailbox.as_deref(),
            Some("contentmailbox@contoso.com")
        );
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
