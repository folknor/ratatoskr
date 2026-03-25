use crate::discovery::types::ProtocolOption;

/// Merge results from all stages, deduplicate, and rank by preference.
pub fn merge_and_rank(stage_results: Vec<Vec<ProtocolOption>>) -> Vec<ProtocolOption> {
    let mut all: Vec<ProtocolOption> = stage_results.into_iter().flatten().collect();

    // Deduplicate: same protocol type + same host (for IMAP) or same session URL (for JMAP)
    dedup(&mut all);

    // Sort by protocol priority first, then by source confidence
    all.sort_by(|a, b| {
        let proto_cmp = a.protocol.priority().cmp(&b.protocol.priority());
        if proto_cmp != std::cmp::Ordering::Equal {
            return proto_cmp;
        }
        a.source.confidence().cmp(&b.source.confidence())
    });

    all
}

/// Re-sort options after post-processing mutations (e.g., OIDC upgrade
/// changes the source, which affects confidence ranking).
pub fn re_rank(options: &mut [ProtocolOption]) {
    options.sort_by(|a, b| {
        let proto_cmp = a.protocol.priority().cmp(&b.protocol.priority());
        if proto_cmp != std::cmp::Ordering::Equal {
            return proto_cmp;
        }
        a.source.confidence().cmp(&b.source.confidence())
    });
}

fn dedup(options: &mut Vec<ProtocolOption>) {
    let mut seen: Vec<String> = Vec::new();

    options.retain(|opt| {
        let key = dedup_key(opt);
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn dedup_key(opt: &ProtocolOption) -> String {
    match &opt.protocol {
        crate::discovery::types::Protocol::GmailApi => "gmail_api".to_string(),
        crate::discovery::types::Protocol::MicrosoftGraph => "microsoft_graph".to_string(),
        crate::discovery::types::Protocol::Jmap { session_url } => {
            format!("jmap:{session_url}")
        }
        crate::discovery::types::Protocol::Imap { incoming, .. } => {
            format!("imap:{}:{}", incoming.hostname, incoming.port)
        }
    }
}
