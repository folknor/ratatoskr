pub mod autoconfig;
pub mod jmap_wellknown;
pub mod merge;
pub mod mx;
pub mod oidc;
pub mod probe;
pub mod registry;
pub mod types;

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use types::{DiscoveredConfig, ProtocolOption, StageDiagnostic, StageOutcome};

/// Overall timeout for the entire discovery operation.
const OVERALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Run the full discovery cascade for an email address.
pub async fn discover(email: &str) -> Result<DiscoveredConfig, String> {
    let email = email.trim().to_lowercase();
    log::info!("Starting email provider discovery for {email}");
    let domain = extract_domain(&email)?;

    match tokio::time::timeout(OVERALL_TIMEOUT, run_cascade(&email, &domain)).await {
        Ok(Ok(config)) => {
            log::info!(
                "Discovery completed for {email}: {} protocol options found",
                config.options.len()
            );
            Ok(config)
        }
        Ok(Err(e)) => {
            log::error!("Discovery failed for {email}: {e}");
            Err(e)
        }
        Err(_) => {
            log::error!("Discovery timed out for {email} after 15 seconds");
            Err("Discovery timed out after 15 seconds".to_string())
        }
    }
}

async fn run_cascade(email: &str, domain: &str) -> Result<DiscoveredConfig, String> {
    let imap_found = Arc::new(Notify::new());
    let (stages_1_4, resolved_domain) = run_stages(domain, email, &imap_found).await;
    let stage_5 = run_probe_stage(domain, &imap_found, &stages_1_4).await;

    let (mut all_results, mut diagnostics) = unpack_stages(stages_1_4);
    all_results.push(stage_5.0);
    diagnostics.push(stage_5.1);

    let options = merge::merge_and_rank(all_results);

    Ok(DiscoveredConfig {
        email: email.to_string(),
        domain: domain.to_string(),
        options,
        resolved_domain,
        diagnostics,
    })
}

fn extract_domain(email: &str) -> Result<String, String> {
    let domain = email
        .split('@')
        .nth(1)
        .ok_or_else(|| "Invalid email address: missing @".to_string())?;
    if domain.is_empty() || !domain.contains('.') {
        return Err("Invalid email address: bad domain".to_string());
    }
    Ok(domain.to_string())
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn make_diag(stage: &'static str, start: Instant, results: &[ProtocolOption]) -> StageDiagnostic {
    log::debug!(
        "Discovery stage '{stage}' completed: {} results in {}ms",
        results.len(),
        elapsed_ms(start)
    );
    let outcome = if results.is_empty() {
        StageOutcome::NotFound
    } else {
        StageOutcome::Found {
            count: results.len(),
        }
    };
    StageDiagnostic {
        stage,
        duration_ms: elapsed_ms(start),
        outcome,
    }
}

type StageResult = (Vec<ProtocolOption>, StageDiagnostic);
type MxResult = (Vec<ProtocolOption>, Option<String>, StageDiagnostic);

async fn run_stages(
    domain: &str,
    email: &str,
    imap_found: &Arc<Notify>,
) -> (
    (StageResult, StageResult, MxResult, StageResult),
    Option<String>,
) {
    let d1 = domain.to_string();
    let s1 = tokio::spawn(async move {
        let start = Instant::now();
        let results = registry::lookup(&d1);
        let diag = make_diag("registry", start, &results);
        (results, diag)
    });

    let (d2, e2) = (domain.to_string(), email.to_string());
    let s2 = tokio::spawn(async move {
        let start = Instant::now();
        let results = autoconfig::fetch(&d2, &e2).await;
        let diag = make_diag("autoconfig", start, &results);
        (results, diag)
    });

    let (d3, e3) = (domain.to_string(), email.to_string());
    let s3 = tokio::spawn(async move {
        let start = Instant::now();
        let (results, resolved) = mx::lookup(&d3, &e3).await;
        let diag = make_diag("mx_lookup", start, &results);
        (results, resolved, diag)
    });

    let d4 = domain.to_string();
    let s4 = tokio::spawn(async move {
        let start = Instant::now();
        let results: Vec<ProtocolOption> = jmap_wellknown::probe(&d4).await.into_iter().collect();
        let diag = make_diag("jmap_wellknown", start, &results);
        (results, diag)
    });

    let (r1, r2, r3, r4) = tokio::join!(s1, s2, s3, s4);

    let r1 = r1.unwrap_or_else(|_| (Vec::new(), err_diag("registry")));
    let r2 = r2.unwrap_or_else(|_| (Vec::new(), err_diag("autoconfig")));
    let r3 = r3.unwrap_or_else(|_| (Vec::new(), None, err_diag("mx_lookup")));
    let r4 = r4.unwrap_or_else(|_| (Vec::new(), err_diag("jmap_wellknown")));

    let resolved_domain = r3.1.clone();

    // Signal port probe if IMAP found
    let has_imap =
        r1.0.iter()
            .chain(r2.0.iter())
            .chain(r3.0.iter())
            .any(|opt| matches!(opt.protocol, types::Protocol::Imap { .. }));
    if has_imap {
        imap_found.notify_one();
    }

    ((r1, r2, r3, r4), resolved_domain)
}

async fn run_probe_stage(
    domain: &str,
    imap_found: &Arc<Notify>,
    _stages: &(StageResult, StageResult, MxResult, StageResult),
) -> StageResult {
    let d = domain.to_string();
    let notify = Arc::clone(imap_found);
    let handle = tokio::spawn(async move {
        let start = Instant::now();
        let cancelled = tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(3)) => false,
            () = notify.notified() => true,
        };
        let results = probe::probe_ports(&d, cancelled).await;
        let outcome = if cancelled {
            StageOutcome::Skipped
        } else if results.is_empty() {
            StageOutcome::NotFound
        } else {
            StageOutcome::Found {
                count: results.len(),
            }
        };
        (
            results,
            StageDiagnostic {
                stage: "port_probe",
                duration_ms: elapsed_ms(start),
                outcome,
            },
        )
    });
    handle
        .await
        .unwrap_or_else(|_| (Vec::new(), err_diag("port_probe")))
}

fn unpack_stages(
    stages: (StageResult, StageResult, MxResult, StageResult),
) -> (Vec<Vec<ProtocolOption>>, Vec<StageDiagnostic>) {
    let (r1, r2, r3, r4) = stages;
    let results = vec![r1.0, r2.0, r3.0, r4.0];
    let diagnostics = vec![r1.1, r2.1, r3.2, r4.1];
    (results, diagnostics)
}

fn err_diag(stage: &'static str) -> StageDiagnostic {
    StageDiagnostic {
        stage,
        duration_ms: 0,
        outcome: StageOutcome::Error {
            message: "task panicked".to_string(),
        },
    }
}
