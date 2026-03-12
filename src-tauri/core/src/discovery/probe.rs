use crate::discovery::types::{
    AuthConfig, AuthMethod, DiscoverySource, Protocol, ProtocolOption, Security, ServerConfig,
    UsernameFormat,
};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_native_tls::TlsConnector;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

struct ProbeCandidate {
    hostname: String,
    port: u16,
    security: Security,
    role: ServerRole,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerRole {
    Imap,
    Smtp,
}

/// Stage 5: Port probing as last resort.
pub async fn probe_ports(domain: &str, cancelled: bool) -> Vec<ProtocolOption> {
    if cancelled {
        return Vec::new();
    }

    let imap_candidates = build_candidates(domain, ServerRole::Imap);
    let smtp_candidates = build_candidates(domain, ServerRole::Smtp);

    let imap_futures: Vec<_> = imap_candidates.into_iter().map(probe_single).collect();
    let smtp_futures: Vec<_> = smtp_candidates.into_iter().map(probe_single).collect();

    let (imap_results, smtp_results) = tokio::join!(
        futures::future::join_all(imap_futures),
        futures::future::join_all(smtp_futures)
    );

    let imap_hit = imap_results.into_iter().flatten().next();
    let smtp_hit = smtp_results.into_iter().flatten().next();

    match (imap_hit, smtp_hit) {
        (Some(imap_cfg), Some(smtp_cfg)) => {
            vec![ProtocolOption {
                protocol: Protocol::Imap {
                    incoming: imap_cfg,
                    outgoing: smtp_cfg,
                },
                auth: AuthConfig {
                    method: AuthMethod::Password,
                    alternatives: Vec::new(),
                },
                provider_name: None,
                help_url: None,
                source: DiscoverySource::PortProbe,
            }]
        }
        _ => Vec::new(),
    }
}

fn build_candidates(domain: &str, role: ServerRole) -> Vec<ProbeCandidate> {
    let prefixes = match role {
        ServerRole::Imap => &["imap.", "mail.", ""][..],
        ServerRole::Smtp => &["smtp.", "mail.", ""][..],
    };
    let ports: &[(u16, Security)] = match role {
        ServerRole::Imap => &[(993, Security::Tls), (143, Security::StartTls)],
        ServerRole::Smtp => &[(587, Security::StartTls), (465, Security::Tls)],
    };

    let mut candidates = Vec::new();
    for prefix in prefixes {
        for &(port, security) in ports {
            candidates.push(ProbeCandidate {
                hostname: format!("{prefix}{domain}"),
                port,
                security,
                role,
            });
        }
    }
    candidates
}

async fn probe_single(candidate: ProbeCandidate) -> Option<ServerConfig> {
    let addr = format!("{}:{}", candidate.hostname, candidate.port);
    let stream = timeout(PROBE_TIMEOUT, TcpStream::connect(&addr))
        .await
        .ok()?
        .ok()?;

    match candidate.security {
        Security::Tls => probe_tls(stream, &candidate).await,
        Security::StartTls => probe_starttls(stream, &candidate).await,
        Security::None => None,
    }
}

async fn probe_tls(stream: TcpStream, candidate: &ProbeCandidate) -> Option<ServerConfig> {
    let connector = native_tls::TlsConnector::new().ok()?;
    let connector = TlsConnector::from(connector);
    let mut tls_stream = connector.connect(&candidate.hostname, stream).await.ok()?;

    let mut banner = vec![0u8; 256];
    let n = timeout(PROBE_TIMEOUT, tls_stream.read(&mut banner))
        .await
        .ok()?
        .ok()?;
    if !validate_banner(&banner[..n], candidate.role) {
        return None;
    }

    Some(ServerConfig {
        hostname: candidate.hostname.clone(),
        port: candidate.port,
        security: candidate.security,
        username: UsernameFormat::EmailAddress,
    })
}

async fn probe_starttls(stream: TcpStream, candidate: &ProbeCandidate) -> Option<ServerConfig> {
    let mut banner = vec![0u8; 256];
    stream.readable().await.ok()?;
    let n = stream.try_read(&mut banner).ok()?;
    if !validate_banner(&banner[..n], candidate.role) {
        return None;
    }

    Some(ServerConfig {
        hostname: candidate.hostname.clone(),
        port: candidate.port,
        security: candidate.security,
        username: UsernameFormat::EmailAddress,
    })
}

fn validate_banner(banner: &[u8], role: ServerRole) -> bool {
    let text = String::from_utf8_lossy(banner);
    match role {
        ServerRole::Imap => text.contains("* OK") || text.contains("* PREAUTH"),
        ServerRole::Smtp => text.starts_with("220"),
    }
}
