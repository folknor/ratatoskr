use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const ROLLED_LOGS: u8 = 3;
static LOGGER: OnceLock<ServiceFileLogger> = OnceLock::new();

pub(crate) fn init(app_data_dir: &Path) -> Result<(), log::SetLoggerError> {
    let logger = ServiceFileLogger::open(app_data_dir).unwrap_or_else(|_| ServiceFileLogger::stderr());
    let _ = LOGGER.set(logger);
    if let Some(logger) = LOGGER.get() {
        log::set_logger(logger)?;
        log::set_max_level(LevelFilter::Info);
        log::info!("service logger initialized");
    }
    Ok(())
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
