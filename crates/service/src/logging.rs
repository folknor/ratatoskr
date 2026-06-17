use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const ROLLED_LOGS: u8 = 3;
const STALE_LOG_AGE: Duration = Duration::from_secs(24 * 60 * 60);
static LOGGER: OnceLock<ServiceFileLogger> = OnceLock::new();

/// Install the rolling-file + stderr logger. On file-open failure (logs
/// directory unwritable, disk full, etc.) the logger degrades to stderr-
/// only and that fact is written directly to stderr so a Service that
/// fails its boot sequence still has at least one channel for the failure
/// cause. We deliberately do not abort here on file-open failure: stderr
/// is inherited on dev runs and captured by the UI's spawn flow in
/// production, so degraded logging is strictly better than no Service.
///
/// Returns `Err(SetLoggerError)` only if `log::set_logger` had already
/// been called in this process. That is unreachable in production
/// (run_service_blocking calls `init` exactly once); the test harness
/// can hit it across multiple in-process Service instances and the error
/// is benign there since the global logger is shared.
pub(crate) fn init(app_data_dir: &Path) -> Result<(), log::SetLoggerError> {
    let (logger, file_open_error) = match ServiceFileLogger::open(app_data_dir) {
        Ok(logger) => (logger, None),
        Err(error) => (ServiceFileLogger::stderr(), Some(error)),
    };
    if let Some(error) = file_open_error {
        // Use stderr().lock() rather than eprintln! so the failure line
        // doesn't race with the panic hook or library code that may also
        // write to stderr. Best-effort: we're already in the degraded
        // path, ignoring this final write means stderr is genuinely
        // unwritable and we have nowhere to surface diagnostics.
        let line = format!(
            "[service] log-file open failed for {} ({error}); falling back to stderr-only logging\n",
            app_data_dir.display(),
        );
        let _ = std::io::Write::write_all(&mut std::io::stderr().lock(), line.as_bytes());
    }
    let _ = LOGGER.set(logger);
    if let Some(logger) = LOGGER.get() {
        log::set_logger(logger)?;
        log::set_max_level(LevelFilter::Info);
        log::info!("service logger initialized");
    }
    Ok(())
}

/// Walk `<app_data>/logs/` and unlink stale `service.<pid>.log*` files that
/// belong to a different PID and whose mtime is older than 24 hours. Bounds
/// the directory size under any path that escapes the terminal-failure
/// policy: even if some future bug produces a tight respawn loop, the per-
/// PID naming would otherwise pile up `service.*.log` files at one per
/// respawn forever.
///
/// Runs after the instance lock is acquired, before DB open. Any errors are
/// logged at `warn` and ignored - this is best-effort housekeeping, not a
/// correctness gate.
pub(crate) fn cleanup_stale_logs(app_data_dir: &Path) {
    cleanup_stale_logs_with(app_data_dir, SystemTime::now(), STALE_LOG_AGE);
}

fn cleanup_stale_logs_with(app_data_dir: &Path, now: SystemTime, max_age: Duration) {
    let log_dir = app_data_dir.join("logs");
    let entries = match fs::read_dir(&log_dir) {
        Ok(entries) => entries,
        // No logs dir means nothing to clean up; logging::init creates it on
        // first run if it doesn't already exist.
        Err(_) => return,
    };
    let current_pid = std::process::id();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(pid) = parse_log_filename_pid(name) else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let age = now.duration_since(modified).unwrap_or_default();
        if age < max_age {
            continue;
        }
        if let Err(error) = fs::remove_file(&path) {
            log::warn!(
                "failed to unlink stale log file {}: {error}",
                path.display()
            );
        }
    }
}

/// Parse the PID out of a service log filename. Returns `None` for the
/// `service.log` symlink, the `service.log.txt` Windows pointer, or any name
/// that doesn't match the `service.<pid>.log[.<n>]` pattern.
fn parse_log_filename_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("service.")?;
    let (pid_str, suffix) = rest.split_once('.')?;
    let pid: u32 = pid_str.parse().ok()?;
    if suffix == "log" || suffix.starts_with("log.") {
        Some(pid)
    } else {
        None
    }
}

pub(crate) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("service panic: {info}");
        default_hook(info);
    }));
}

struct ServiceFileLogger {
    file: Option<Mutex<File>>,
    path: Option<PathBuf>,
    stderr: bool,
}

impl ServiceFileLogger {
    fn open(app_data_dir: &Path) -> std::io::Result<Self> {
        let log_dir = app_data_dir.join("logs");
        fs::create_dir_all(&log_dir)?;
        let path = log_dir.join(format!("service.{}.log", std::process::id()));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        update_current_pointer(&log_dir, &path);
        Ok(Self {
            file: Some(Mutex::new(file)),
            path: Some(path),
            stderr: true,
        })
    }

    fn stderr() -> Self {
        Self {
            file: None,
            path: None,
            stderr: true,
        }
    }

    fn write_file(&self, line: &str) {
        let (Some(file), Some(path)) = (&self.file, &self.path) else {
            return;
        };
        let Ok(mut guard) = file.lock() else {
            return;
        };
        if guard.metadata().map(|meta| meta.len()).unwrap_or(0) > MAX_LOG_BYTES {
            rotate(path, &mut guard);
        }
        let _ = guard.write_all(line.as_bytes());
    }
}

impl Log for ServiceFileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "[service] {} {} - {}\n",
            record.level(),
            record.target(),
            record.args()
        );
        self.write_file(&line);
        if self.stderr {
            eprint!("{line}");
        }
    }

    fn flush(&self) {}
}

fn rotate(path: &Path, file: &mut File) {
    let _ = file.flush();
    for index in (1..ROLLED_LOGS).rev() {
        let from = path.with_extension(format!("log.{index}"));
        let to = path.with_extension(format!("log.{}", index + 1));
        if from.exists() {
            let _ = fs::rename(from, to);
        }
    }
    let first = path.with_extension("log.1");
    let _ = fs::rename(path, first);
    if let Ok(new_file) = OpenOptions::new().create(true).append(true).open(path) {
        *file = new_file;
    }
}

#[cfg(unix)]
fn update_current_pointer(log_dir: &Path, path: &Path) {
    use std::os::unix::fs::symlink;

    let current = log_dir.join("service.log");
    let _ = fs::remove_file(&current);
    let _ = symlink(path, current);
}

#[cfg(windows)]
fn update_current_pointer(log_dir: &Path, path: &Path) {
    let pointer = log_dir.join("service.log.txt");
    let _ = fs::write(pointer, path.display().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir(suffix: &str) -> std::io::Result<PathBuf> {
        let path = std::env::current_dir()?.join("target").join(format!(
            "logging-cleanup-{}-{}-{}",
            std::process::id(),
            suffix,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("logs"))?;
        Ok(path)
    }

    fn write_with_mtime(path: &Path, mtime: SystemTime) -> std::io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        file.set_modified(mtime)?;
        Ok(())
    }

    #[test]
    fn parse_log_filename_pid_recognises_log_and_rolled_log() {
        assert_eq!(parse_log_filename_pid("service.1234.log"), Some(1234));
        assert_eq!(parse_log_filename_pid("service.1234.log.1"), Some(1234));
        assert_eq!(parse_log_filename_pid("service.1234.log.42"), Some(1234));
    }

    #[test]
    fn parse_log_filename_pid_rejects_non_log_names() {
        // Symlink / pointer files should NOT be matched.
        assert_eq!(parse_log_filename_pid("service.log"), None);
        assert_eq!(parse_log_filename_pid("service.log.txt"), None);
        // Non-service files.
        assert_eq!(parse_log_filename_pid("random.txt"), None);
        assert_eq!(parse_log_filename_pid("service.foo.log"), None);
        assert_eq!(parse_log_filename_pid("notservice.1234.log"), None);
    }

    #[test]
    fn cleanup_removes_stale_other_pid_logs() {
        let dir = temp_data_dir("stale_removed").expect("temp dir");
        let logs = dir.join("logs");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(48 * 60 * 60);

        let stale_main = logs.join("service.999999.log");
        let stale_rolled = logs.join("service.999999.log.1");
        write_with_mtime(&stale_main, old).expect("write stale main");
        write_with_mtime(&stale_rolled, old).expect("write stale rolled");

        cleanup_stale_logs_with(&dir, now, Duration::from_secs(24 * 60 * 60));

        assert!(
            !stale_main.exists(),
            "stale main log should have been unlinked"
        );
        assert!(
            !stale_rolled.exists(),
            "stale rolled log should have been unlinked"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_keeps_recent_other_pid_logs() {
        let dir = temp_data_dir("recent_kept").expect("temp dir");
        let logs = dir.join("logs");
        let now = SystemTime::now();
        let recent = now - Duration::from_secs(60 * 60);

        let recent_main = logs.join("service.999999.log");
        write_with_mtime(&recent_main, recent).expect("write recent main");

        cleanup_stale_logs_with(&dir, now, Duration::from_secs(24 * 60 * 60));

        assert!(
            recent_main.exists(),
            "recent log (1h old) must be kept under 24h cap"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_keeps_current_pid_logs_regardless_of_age() {
        let dir = temp_data_dir("current_kept").expect("temp dir");
        let logs = dir.join("logs");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(10 * 24 * 60 * 60);

        let current = logs.join(format!("service.{}.log", std::process::id()));
        write_with_mtime(&current, old).expect("write current");

        cleanup_stale_logs_with(&dir, now, Duration::from_secs(24 * 60 * 60));

        assert!(
            current.exists(),
            "current PID's log must be kept even if old (the active writer is using it)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_keeps_pointer_files_unconditionally() {
        let dir = temp_data_dir("pointer_kept").expect("temp dir");
        let logs = dir.join("logs");
        let now = SystemTime::now();
        let ancient = now - Duration::from_secs(365 * 24 * 60 * 60);

        let unix_pointer = logs.join("service.log");
        let windows_pointer = logs.join("service.log.txt");
        let unrelated = logs.join("random.txt");
        write_with_mtime(&unix_pointer, ancient).expect("write unix pointer");
        write_with_mtime(&windows_pointer, ancient).expect("write windows pointer");
        write_with_mtime(&unrelated, ancient).expect("write unrelated");

        cleanup_stale_logs_with(&dir, now, Duration::from_secs(24 * 60 * 60));

        assert!(
            unix_pointer.exists(),
            "service.log pointer must never be unlinked"
        );
        assert!(
            windows_pointer.exists(),
            "service.log.txt pointer must never be unlinked"
        );
        assert!(unrelated.exists(), "non-service files must not be touched");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_on_missing_logs_dir_is_a_noop() {
        let dir = temp_data_dir("no_logs_dir").expect("temp dir");
        // Remove the logs subdir to simulate a fresh data dir before any
        // logger has run.
        fs::remove_dir_all(dir.join("logs")).expect("remove logs dir");
        // Must not panic, must not error - it's best-effort housekeeping.
        cleanup_stale_logs_with(&dir, SystemTime::now(), Duration::from_secs(24 * 60 * 60));
        let _ = fs::remove_dir_all(&dir);
    }
}
