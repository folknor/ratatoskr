use crate::discovery::registry;
use crate::discovery::types::{
    AuthConfig, AuthMethod, DiscoverySource, Protocol, ProtocolOption, Security, ServerConfig,
    UsernameFormat,
};
use quick_xml::Reader;
use quick_xml::events::Event;

/// Stage 2: Fetch and parse Mozilla autoconfig XML.
pub async fn fetch(domain: &str, email: &str) -> Vec<ProtocolOption> {
    let urls = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}"),
        format!(
            "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={email}"
        ),
    ];

    for url in &urls {
        if let Some(options) = try_fetch_and_parse(url, email).await
            && !options.is_empty()
        {
            return options;
        }
    }
    Vec::new()
}

async fn try_fetch_and_parse(url: &str, email: &str) -> Option<Vec<ProtocolOption>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let body = resp.text().await.ok()?;
    Some(parse_autoconfig_xml(&body, email, url))
}

struct ParsedServer {
    server_type: String,
    hostname: String,
    port: u16,
    socket_type: String,
    authentication: String,
    username_template: String,
}

fn parse_autoconfig_xml(xml: &str, email: &str, source_url: &str) -> Vec<ProtocolOption> {
    let mut reader = Reader::from_str(xml);
    let mut servers: Vec<ParsedServer> = Vec::new();
    let mut display_name: Option<String> = None;
    let mut current_server: Option<ParsedServer> = None;
    let mut current_tag = String::new();
    let mut in_email_provider = false;
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                handle_start_tag(&name, e, in_email_provider, &mut current_server);
                if name == "emailProvider" {
                    in_email_provider = true;
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
                handle_end_tag(
                    &name,
                    &current_tag,
                    &buf,
                    in_email_provider,
                    &mut current_server,
                    &mut servers,
                    &mut display_name,
                );
                if name == "emailProvider" {
                    in_email_provider = false;
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    build_options_from_servers(&servers, email, source_url, display_name.as_deref())
}

fn handle_start_tag(
    name: &str,
    e: &quick_xml::events::BytesStart<'_>,
    in_email_provider: bool,
    current_server: &mut Option<ParsedServer>,
) {
    if matches!(name, "incomingServer" | "outgoingServer") && in_email_provider {
        let server_type = e
            .attributes()
            .filter_map(std::result::Result::ok)
            .find(|a| a.key.as_ref() == b"type")
            .map(|a| String::from_utf8_lossy(&a.value).to_string())
            .unwrap_or_default();
        *current_server = Some(ParsedServer {
            server_type,
            hostname: String::new(),
            port: 0,
            socket_type: String::new(),
            authentication: String::new(),
            username_template: String::new(),
        });
    }
}

fn handle_end_tag(
    name: &str,
    current_tag: &str,
    buf: &str,
    in_email_provider: bool,
    current_server: &mut Option<ParsedServer>,
    servers: &mut Vec<ParsedServer>,
    display_name: &mut Option<String>,
) {
    if let Some(server) = current_server {
        let trimmed = buf.trim().to_string();
        match current_tag {
            "hostname" => server.hostname = trimmed,
            "port" => server.port = trimmed.parse().unwrap_or(0),
            "socketType" => server.socket_type = trimmed,
            "authentication" => server.authentication = trimmed,
            "username" => server.username_template = trimmed,
            _ => {}
        }
    }

    match name {
        "displayName" if in_email_provider && current_server.is_none() => {
            *display_name = Some(buf.trim().to_string());
        }
        "incomingServer" | "outgoingServer" => {
            if let Some(server) = current_server.take()
                && !server.hostname.is_empty()
                && server.port > 0
            {
                servers.push(server);
            }
        }
        _ => {}
    }
}

fn build_options_from_servers(
    servers: &[ParsedServer],
    email: &str,
    source_url: &str,
    display_name: Option<&str>,
) -> Vec<ProtocolOption> {
    let local_part = email.split('@').next().unwrap_or(email);
    let email_domain = email.split('@').nth(1).unwrap_or("");

    let imap_server = servers.iter().find(|s| s.server_type == "imap");
    let smtp_server = servers.iter().find(|s| s.server_type == "smtp");

    let mut options = Vec::new();

    if let (Some(imap), Some(smtp)) = (imap_server, smtp_server) {
        let auth = resolve_auth(imap, email_domain);
        let source = DiscoverySource::AutoconfigXml {
            url: source_url.to_string(),
        };

        options.push(ProtocolOption {
            protocol: Protocol::Imap {
                incoming: ServerConfig {
                    hostname: substitute_vars(&imap.hostname, email, local_part, email_domain),
                    port: imap.port,
                    security: parse_security(&imap.socket_type),
                    username: parse_username_format(&imap.username_template),
                },
                outgoing: ServerConfig {
                    hostname: substitute_vars(&smtp.hostname, email, local_part, email_domain),
                    port: smtp.port,
                    security: parse_security(&smtp.socket_type),
                    username: parse_username_format(&smtp.username_template),
                },
            },
            auth,
            provider_name: display_name.map(str::to_string),
            source,
        });
    }

    options
}

fn substitute_vars(template: &str, email: &str, local_part: &str, domain: &str) -> String {
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local_part)
        .replace("%EMAILDOMAIN%", domain)
}

fn parse_security(socket_type: &str) -> Security {
    match socket_type.to_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => Security::None,
    }
}

fn parse_username_format(template: &str) -> UsernameFormat {
    match template {
        "%EMAILLOCALPART%" => UsernameFormat::LocalPart,
        "%EMAILADDRESS%" | "" => UsernameFormat::EmailAddress,
        other => UsernameFormat::Custom {
            value: other.to_string(),
        },
    }
}

fn resolve_auth(server: &ParsedServer, domain: &str) -> AuthConfig {
    let auth_lower = server.authentication.to_lowercase();
    if auth_lower == "oauth2" || auth_lower == "xoauth2" {
        if let Some(oauth_method) = registry::lookup_oauth_for_domain(domain) {
            return AuthConfig {
                method: oauth_method,
                alternatives: Vec::new(),
            };
        }
        return AuthConfig {
            method: AuthMethod::OAuth2Unsupported {
                provider_domain: domain.to_string(),
            },
            alternatives: Vec::new(),
        };
    }
    AuthConfig {
        method: AuthMethod::Password,
        alternatives: Vec::new(),
    }
}
