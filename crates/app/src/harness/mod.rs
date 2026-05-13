#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    reason = "Lua's public numeric ABI is f64; harness scripts use bounded test values"
)]

use crate::service_client::{
    ClientError, ServiceClient, ServiceNotificationReceiver, ServiceTraceSink, SpawnEvent,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use dellingr::error::ErrorKind;
use dellingr::{ArgCount, LuaType, RetCount, State};
use service_api::{
    AccountDeleteParams, ActionWireOperation, ActionWirePlan, AttachmentFetchParams,
    BootClassification, BootExitCode, BootPhaseKind, CalendarActionPlan, CalendarActionWireOperation,
    ClientNotification, ContactDeleteParams, ContactSaveParams, ExtractStatusParams,
    IndexRebuildParams, Notification, OauthExchangeCodeParams, OperationId, PlanId,
    ReadBootstrapSnapshotsParams, RebuildPolicy, RedactedString, RequestParams,
    SendAttachmentSource, SendWireAttachment, SendWireMessage, SendWireRequest, SettingValue,
    SettingsSetParams, TestCrashAfterNWritesParams, TestDelayNextWriteParams,
    TestPendingOpsReadParams, TestQueryDbStateParams, TestSeedAccountParams,
    TestRemoveCachedAttachmentBytesParams, TestSeedCachedAttachmentParams,
    TestSeedRemoteAttachmentParams, TestSearchIndexParams, TestSeedThreadParams,
    TestStartSyncParams, TestThreadReadParams, WireCalendarEventInput, WireCalendarOperation,
    WireFolderId, WireMailOperation, WireTagId,
};
use std::collections::HashMap;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

type HarnessResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const TEST_KEY: &str = "paWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaU=";

pub fn run(script: PathBuf) -> HarnessResult {
    let _ = env_logger::try_init();
    let script_source = std::fs::read_to_string(&script)?;
    let ceiling = parse_ceiling(&script_source).unwrap_or(Duration::from_secs(60));
    let artefact_dir = artefact_dir()?;
    std::fs::create_dir_all(&artefact_dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let trace = ServiceTraceSink::new(&artefact_dir)?;
    let context = Arc::new(Mutex::new(HarnessContext::new(
        runtime.handle().clone(),
        app_binary_path()?,
        artefact_dir,
        trace,
    )?));
    let thread_context = Arc::clone(&context);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run_script(script, script_source, thread_context);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(ceiling) {
        Ok(result) => {
            let success = result.is_ok();
            finish_context(&context, success, result.as_ref().err().map(ToString::to_string));
            result
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let message = format!("script exceeded wall-clock ceiling of {ceiling:?}");
            finish_context(&context, false, Some(message.clone()));
            Err(message.into())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let message = "harness script thread exited without reporting a result".to_string();
            finish_context(&context, false, Some(message.clone()));
            Err(message.into())
        }
    }
}

fn run_script(
    script: PathBuf,
    source: String,
    context: Arc<Mutex<HarnessContext>>,
) -> HarnessResult {
    let mut state = State::new();
    state.set_user_data(context);
    install_globals(&mut state).map_err(box_lua_error)?;
    state
        .load_string_named(source, Some(script.display().to_string()))
        .map_err(box_lua_error)?;
    state
        .call(ArgCount::Fixed(0), RetCount::Fixed(0))
        .map_err(box_lua_error)?;
    Ok(())
}

fn install_globals(state: &mut State) -> dellingr::Result<()> {
    state.new_table();
    let table_idx = state.get_top() as isize;
    set_field_fn(state, table_idx, "data_dir", lua_data_dir)?;
    set_field_fn(state, table_idx, "spawn", lua_spawn)?;
    set_field_fn(state, table_idx, "spawn_with_events", lua_spawn_with_events)?;
    set_field_fn(
        state,
        table_idx,
        "spawn_parent_death_helper",
        lua_spawn_parent_death_helper,
    )?;
    set_field_fn(state, table_idx, "kill", lua_kill)?;
    set_field_fn(state, table_idx, "pid_is_alive", lua_pid_is_alive)?;
    set_field_fn(state, table_idx, "path_exists", lua_path_exists)?;
    set_field_fn(state, table_idx, "dir_has_prefix", lua_dir_has_prefix)?;
    set_field_fn(state, table_idx, "read_json", lua_read_json)?;
    set_field_fn(state, table_idx, "read_text", lua_read_text)?;
    set_field_fn(state, table_idx, "read_base64", lua_read_base64)?;
    set_field_fn(state, table_idx, "write_text", lua_write_text)?;
    set_field_fn(state, table_idx, "write_summary", lua_write_summary)?;
    set_field_fn(state, table_idx, "sleep", lua_sleep)?;
    set_field_fn(state, table_idx, "now_ms", lua_now_ms)?;
    set_field_fn(state, table_idx, "marker", lua_marker)?;
    set_field_fn(state, table_idx, "uuid", lua_uuid)?;
    set_field_fn(state, table_idx, "repeat_byte", lua_repeat_byte)?;
    set_field_fn(state, table_idx, "stage_attachment", lua_stage_attachment)?;
    set_field_fn(state, table_idx, "assert", lua_assert)?;
    set_field_fn(state, table_idx, "assert_eq", lua_assert_eq)?;
    set_field_fn(state, table_idx, "same_client", lua_same_client)?;
    set_field_fn(state, table_idx, "expect_quiet", lua_expect_quiet)?;
    set_field_fn(state, table_idx, "join_url", lua_join_url)?;
    set_field_fn(state, table_idx, "mock_requests", lua_mock_requests)?;
    set_field_fn(state, table_idx, "snapshot_state", lua_snapshot_state)?;
    set_field_fn(state, table_idx, "latency", lua_latency)?;
    set_field_fn(state, table_idx, "set_latency", lua_set_latency)?;
    set_field_fn(
        state,
        table_idx,
        "clear_mock_requests",
        lua_clear_mock_requests,
    )?;
    set_field_fn(state, table_idx, "request_count", lua_request_count)?;
    set_field_fn(
        state,
        table_idx,
        "request_count_prefix",
        lua_request_count_prefix,
    )?;
    set_field_fn(state, table_idx, "http_json", lua_http_json)?;
    set_field_fn(state, table_idx, "http_get", lua_http_get)?;
    set_field_fn(state, table_idx, "http_post_json", lua_http_post_json)?;
    set_field_fn(state, table_idx, "http_delete", lua_http_delete)?;
    set_field_fn(state, table_idx, "http", lua_http)?;
    set_field_fn(state, table_idx, "env", lua_env)?;
    set_field_number(state, table_idx, "protocol_version", service_api::PROTOCOL_VERSION as f64)?;
    state.set_global("harness");
    Ok(())
}

struct HarnessContext {
    handle: tokio::runtime::Handle,
    app_binary: PathBuf,
    artefact_dir: PathBuf,
    trace: Arc<ServiceTraceSink>,
    resources: HashMap<u64, HarnessResource>,
    next_id: u64,
    data_dirs: Vec<PathBuf>,
    last_pid: Option<u32>,
    started: Instant,
    events: std::fs::File,
    steps: std::fs::File,
}

impl HarnessContext {
    fn new(
        handle: tokio::runtime::Handle,
        app_binary: PathBuf,
        artefact_dir: PathBuf,
        trace: Arc<ServiceTraceSink>,
    ) -> std::io::Result<Self> {
        let events = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(artefact_dir.join("events.jsonl"))?;
        let steps = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(artefact_dir.join("steps.jsonl"))?;
        Ok(Self {
            handle,
            app_binary,
            artefact_dir,
            trace,
            resources: HashMap::new(),
            next_id: 1,
            data_dirs: Vec::new(),
            last_pid: None,
            started: Instant::now(),
            events,
            steps,
        })
    }

    fn insert(&mut self, resource: HarnessResource) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.resources.insert(id, resource);
        id
    }

    fn remove(&mut self, id: u64) {
        self.resources.remove(&id);
    }

    fn client(&self, id: u64) -> dellingr::Result<Arc<ServiceClient>> {
        match self.resources.get(&id) {
            Some(HarnessResource::Client(client)) => Ok(Arc::clone(client)),
            _ => Err(lua_error_message(format!("no client resource {id}"))),
        }
    }

    fn record_step(&mut self, step: &str, kind: &str, transition: &str) {
        let record = serde_json::json!({
            "schema": 1,
            "ts_ms": self.started.elapsed().as_millis(),
            "step": step,
            "kind": kind,
            "transition": transition,
        });
        let _ = writeln!(self.steps, "{record}");
        let _ = self.steps.flush();
    }

    fn record_event(&mut self, event: &serde_json::Value) {
        let record = serde_json::json!({
            "schema": 1,
            "ts_ms": self.started.elapsed().as_millis(),
            "event": event,
        });
        let _ = writeln!(self.events, "{record}");
        let _ = self.events.flush();
    }

    fn finish(&mut self, success: bool, error: Option<String>) {
        if !success {
            if let Some(pid) = self.last_pid {
                self.snapshot_proc(pid);
            }
            self.copy_data_dirs();
        }
        let record = serde_json::json!({
            "schema": 1,
            "success": success,
            "error": error,
            "elapsed_ms": self.started.elapsed().as_millis(),
        });
        let _ = std::fs::write(
            self.artefact_dir.join("runtime-outcome.json"),
            format!("{record}\n"),
        );
    }

    fn copy_data_dirs(&self) {
        let snapshot_root = self.artefact_dir.join("data-dir");
        let _ = std::fs::remove_dir_all(&snapshot_root);
        let _ = std::fs::create_dir_all(&snapshot_root);
        for dir in &self.data_dirs {
            let Some(name) = dir.file_name() else {
                continue;
            };
            let _ = copy_dir_recursive(dir, &snapshot_root.join(name));
        }
    }

    fn snapshot_proc(&self, pid: u32) {
        #[cfg(target_os = "linux")]
        {
            let proc_root = PathBuf::from("/proc").join(pid.to_string());
            for name in ["status", "wchan", "syscall", "stack"] {
                let src = proc_root.join(name);
                let dst = self.artefact_dir.join(format!("proc-{name}.txt"));
                match std::fs::read_to_string(&src) {
                    Ok(contents) => {
                        let _ = std::fs::write(dst, contents);
                    }
                    Err(error) => {
                        let _ = std::fs::write(dst, format!("unavailable: {error}\n"));
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
        }
    }
}

enum HarnessResource {
    Client(Arc<ServiceClient>),
    Events(mpsc::Receiver<SpawnEvent>),
    Notifications(ServiceNotificationReceiver),
    Request(tokio::task::JoinHandle<Result<serde_json::Value, ClientError>>),
    /// A sibling-binary child the harness drives directly (parent_death_helper).
    /// Held here so kill_on_drop fires on teardown and SIGCHLD reaps the child
    /// through the tokio runtime before the harness process exits. The Child
    /// itself is never read after insertion - it exists for Drop side effects.
    Helper(#[allow(dead_code)] tokio::process::Child),
}

fn lua_data_dir(state: &mut State) -> dellingr::Result<u8> {
    let suffix = state.to_string(1)?;
    let with_key = if state.get_top() >= 2 {
        state.to_boolean(2)
    } else {
        true
    };
    let ctx = context(state)?;
    let path = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let root = guard.artefact_dir.join("data").join(&suffix);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(lua_io)?;
        if with_key {
            std::fs::write(root.join("ratatoskr.key"), TEST_KEY).map_err(lua_io)?;
        }
        guard.data_dirs.push(root.clone());
        guard.record_step("data_dir", "fixture", "created");
        root
    };
    state.set_top(0);
    state.push_string(path.display().to_string());
    Ok(1)
}

fn lua_spawn(state: &mut State) -> dellingr::Result<u8> {
    let data_dir = PathBuf::from(state.to_string(1)?);
    let extra_args = read_extra_args(state, 2)?;
    let ctx = context(state)?;
    let (handle, binary, trace) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (
            guard.handle.clone(),
            guard.app_binary.clone(),
            Some(Arc::clone(&guard.trace)),
        )
    };
    let refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
    let result = handle.block_on(ServiceClient::spawn_for_harness(
        &binary,
        &data_dir,
        &refs,
        trace,
    ));
    state.set_top(0);
    match result {
        Ok(client) => {
            let id = {
                let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
                guard.last_pid = client.child_pid();
                guard.record_step("spawn", "spawn", "ok");
                guard.insert(HarnessResource::Client(client))
            };
            push_client_table(state, id)?;
            state.push_nil();
        }
        Err(error) => {
            state.push_nil();
            push_client_error(state, &error)?;
        }
    }
    Ok(2)
}

fn lua_spawn_with_events(state: &mut State) -> dellingr::Result<u8> {
    let data_dir = PathBuf::from(state.to_string(1)?);
    let extra_args = read_extra_args(state, 2)?;
    let ctx = context(state)?;
    let (handle, binary, trace) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (
            guard.handle.clone(),
            guard.app_binary.clone(),
            Some(Arc::clone(&guard.trace)),
        )
    };
    let _runtime_enter = handle.enter();
    let rx = ServiceClient::spawn_with_events_for_harness(binary, data_dir, extra_args, trace);
    let id = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.record_step("spawn_with_events", "spawn", "started");
        guard.insert(HarnessResource::Events(rx))
    };
    state.set_top(0);
    push_events_table(state, id)?;
    Ok(1)
}

/// Spawn `parent_death_helper` as a child of the harness, which in turn
/// spawns the Service with `PR_SET_PDEATHSIG = SIGTERM` set via
/// `pre_exec`. The helper prints the Service's pid to stdout and sleeps;
/// the Lua test then SIGKILLs the helper and polls the Service pid.
///
/// Returns a Lua table `{ helper_pid, service_pid }`. The tokio Child
/// handle is held in `HarnessResource::Helper` for the lifetime of the
/// harness context so `kill_on_drop` fires on teardown and tokio's
/// runtime reaps the zombie before the harness binary exits.
///
/// Linux-only - the helper's `main` bails non-zero on other platforms,
/// so the read-pid step will fail with EOF.
fn lua_spawn_parent_death_helper(state: &mut State) -> dellingr::Result<u8> {
    let data_dir = PathBuf::from(state.to_string(1)?);
    let ctx = context(state)?;
    let (handle, app_binary) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.app_binary.clone())
    };
    let helper_binary = parent_death_helper_path(&app_binary)?;

    // tokio::process::Command::spawn requires the tokio runtime to be in
    // scope; the BufReader read for the pid line is async. Wrap both in
    // a single block_on so the runtime context is correct for spawn AND
    // the read.
    let (child, helper_pid, service_pid) = handle
        .block_on(async move {
            let mut child = tokio::process::Command::new(&helper_binary)
                .arg(&app_binary)
                .arg(&data_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .kill_on_drop(true)
                .spawn()?;
            let helper_pid = child
                .id()
                .ok_or_else(|| std::io::Error::other("parent_death_helper has no pid"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| std::io::Error::other("parent_death_helper has no stdout"))?;
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                reader.read_line(&mut line),
            )
            .await
            .map_err(|_| {
                std::io::Error::other("parent_death_helper did not print pid in time")
            })??;
            let service_pid = line.trim().parse::<u32>().map_err(|e| {
                std::io::Error::other(format!("parse helper pid {line:?}: {e}"))
            })?;
            Ok::<_, std::io::Error>((child, helper_pid, service_pid))
        })
        .map_err(lua_io)?;

    {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.record_step("spawn_parent_death_helper", "spawn", "ok");
        let _ = guard.insert(HarnessResource::Helper(child));
    }

    state.set_top(0);
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_number(state, idx, "helper_pid", helper_pid as f64)?;
    set_field_number(state, idx, "service_pid", service_pid as f64)?;
    Ok(1)
}

fn parent_death_helper_path(app_binary: &Path) -> dellingr::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("BROKKR_TEST_BIN_DIR") {
        let candidate = PathBuf::from(dir).join("parent_death_helper");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    if let Some(parent) = app_binary.parent() {
        let candidate = parent.join("parent_death_helper");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(lua_io(std::io::Error::other(
        "parent_death_helper binary not found alongside app",
    )))
}

fn lua_kill(state: &mut State) -> dellingr::Result<u8> {
    let pid = state.to_number(1)? as i32;
    let signal = signal_number(state)?;
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid, signal) };
        if result != 0 {
            return Err(lua_io(std::io::Error::last_os_error()));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        return Err(lua_error_message("harness.kill is only implemented on unix"));
    }
    if let Ok(ctx) = context(state) {
        ctx.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record_step("kill", "process", "sent");
    }
    state.set_top(0);
    state.push_boolean(true);
    Ok(1)
}

fn lua_pid_is_alive(state: &mut State) -> dellingr::Result<u8> {
    let pid = state.to_number(1)? as u32;
    state.set_top(0);
    state.push_boolean(pid_is_alive(pid).map_err(lua_io)?);
    Ok(1)
}

fn lua_path_exists(state: &mut State) -> dellingr::Result<u8> {
    let path = PathBuf::from(state.to_string(1)?);
    state.set_top(0);
    state.push_boolean(path.exists());
    Ok(1)
}

fn lua_dir_has_prefix(state: &mut State) -> dellingr::Result<u8> {
    let dir = PathBuf::from(state.to_string(1)?);
    let prefix = state.to_string(2)?;
    let exists = std::fs::read_dir(dir)
        .map(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(&prefix))
            })
        })
        .unwrap_or(false);
    state.set_top(0);
    state.push_boolean(exists);
    Ok(1)
}

fn lua_read_json(state: &mut State) -> dellingr::Result<u8> {
    let path = PathBuf::from(state.to_string(1)?);
    let text = std::fs::read_to_string(path).map_err(lua_io)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(lua_json)?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_read_text(state: &mut State) -> dellingr::Result<u8> {
    let path = PathBuf::from(state.to_string(1)?);
    let text = std::fs::read_to_string(path).map_err(lua_io)?;
    state.set_top(0);
    state.push_string(text);
    Ok(1)
}

fn lua_read_base64(state: &mut State) -> dellingr::Result<u8> {
    let path = PathBuf::from(state.to_string(1)?);
    let bytes = std::fs::read(path).map_err(lua_io)?;
    state.set_top(0);
    state.push_string(STANDARD.encode(bytes));
    Ok(1)
}

fn lua_write_text(state: &mut State) -> dellingr::Result<u8> {
    let path = PathBuf::from(state.to_string(1)?);
    let text = state.to_string(2)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(lua_io)?;
    }
    std::fs::write(path, text).map_err(lua_io)?;
    state.set_top(0);
    Ok(0)
}

fn lua_write_summary(state: &mut State) -> dellingr::Result<u8> {
    if state.typ(1) != LuaType::Table {
        return Err(lua_error_message("write_summary requires summary table"));
    }
    let value = lua_table_to_json(state, 1)?;
    if !value.is_object() {
        return Err(lua_error_message("write_summary root must be a JSON object"));
    }
    let text = serde_json::to_string_pretty(&value).map_err(lua_json)?;
    let path = {
        let ctx = context(state)?;
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.artefact_dir.join("summary.json")
    };
    std::fs::write(&path, text).map_err(lua_io)?;
    state.set_top(0);
    state.push_string(path.display().to_string());
    Ok(1)
}

fn lua_sleep(state: &mut State) -> dellingr::Result<u8> {
    let millis = state.to_number(1)? as u64;
    std::thread::sleep(Duration::from_millis(millis));
    state.set_top(0);
    Ok(0)
}

fn lua_now_ms(state: &mut State) -> dellingr::Result<u8> {
    let elapsed = {
        let ctx = context(state)?;
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.started.elapsed().as_millis()
    };
    state.set_top(0);
    state.push_number(elapsed as f64);
    Ok(1)
}

fn lua_marker(state: &mut State) -> dellingr::Result<u8> {
    let name = state.to_string(1)?;
    validate_sidecar_marker_name(&name)?;
    let emitted = emit_sidecar_marker(&name)?;
    state.set_top(0);
    state.push_boolean(emitted);
    Ok(1)
}

fn validate_sidecar_marker_name(name: &str) -> dellingr::Result<()> {
    if name.is_empty() {
        return Err(lua_error_message("marker name must not be empty"));
    }
    if name.starts_with('@') {
        return Err(lua_error_message(
            "marker name must not start with @; @ is reserved for sidecar counters",
        ));
    }
    if name.contains('\n') || name.contains('\r') {
        return Err(lua_error_message("marker name must not contain newlines"));
    }
    Ok(())
}

fn emit_sidecar_marker(name: &str) -> dellingr::Result<bool> {
    let Ok(path) = std::env::var("BROKKR_MARKER_FIFO") else {
        return Ok(false);
    };
    if path.is_empty() {
        return Ok(false);
    }
    let timestamp_us = sidecar_timestamp_us()?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(lua_io)?;
    writeln!(file, "{timestamp_us} {name}").map_err(lua_io)?;
    Ok(true)
}

fn sidecar_timestamp_us() -> dellingr::Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| lua_error_message(format!("system clock before UNIX_EPOCH: {error}")))?;
    i64::try_from(elapsed.as_micros())
        .map_err(|_| lua_error_message("sidecar marker timestamp overflow"))
}

fn lua_uuid(state: &mut State) -> dellingr::Result<u8> {
    state.set_top(0);
    state.push_string(PlanId::new_v7().to_string());
    Ok(1)
}

/// `harness.repeat_byte(b, n)` -> string of length `n`, every byte equal
/// to the first byte of `b`.
///
/// ASCII-only by construction: `dellingr::push_string` takes `&str`, so
/// the produced buffer must be valid UTF-8. The cheapest UTF-8 guarantee
/// is to require `b[0] < 0x80` - repeating any single ASCII byte yields
/// valid UTF-8. High bytes are rejected up front with a clear message
/// rather than falling through to a late `from_utf8` error. Test scripts
/// only need bulk filler ("z" * 5MB), so the ASCII restriction is not
/// limiting in practice.
fn lua_repeat_byte(state: &mut State) -> dellingr::Result<u8> {
    let value = state.to_string(1)?;
    let byte = value
        .as_bytes()
        .first()
        .copied()
        .ok_or_else(|| lua_error_message("repeat_byte requires a non-empty byte string"))?;
    if byte >= 0x80 {
        return Err(lua_error_message(format!(
            "repeat_byte requires an ASCII byte (< 0x80), got 0x{byte:02x}"
        )));
    }
    let len = state.to_number(2)? as usize;
    // byte < 0x80, so `vec![byte; len]` is valid UTF-8.
    let repeated = String::from_utf8(vec![byte; len])
        .map_err(|error| lua_error_message(error.to_string()))?;
    state.set_top(0);
    state.push_string(repeated);
    Ok(1)
}

fn lua_stage_attachment(state: &mut State) -> dellingr::Result<u8> {
    let app_data_dir = PathBuf::from(state.to_string(1)?);
    let send_id_value = state.to_string(2)?;
    let send_id = parse_plan_id(&send_id_value)?;
    let index = state.to_number(3)? as usize;
    let relative_path = format!("{index}.bin");
    let staging_dir = app_data_dir
        .join("staging")
        .join(send_id.to_string());
    std::fs::create_dir_all(&staging_dir).map_err(lua_io)?;
    // 50 MB attachments hit this path - wrap the file in a BufWriter so the
    // 8 KiB chunk loop below collapses to a handful of syscalls instead of
    // ~6400. The wrapper also covers the small-payload branch for free.
    let file = std::fs::File::create(staging_dir.join(&relative_path)).map_err(lua_io)?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    // BLAKE3 to match `service::send_vault::verify_and_transfer`,
    // which re-hashes the staged file with `blake3_file` and rejects
    // any hash that does not match. The harness previously hashed
    // with SHA-256, which mismatched on every staged send.
    let mut hasher = blake3::Hasher::new();
    let size = if state.typ(4) == LuaType::Table {
        let len = get_number_field(state, 4, "size")?
            .ok_or_else(|| lua_error_message("stage_attachment payload requires size"))?
            as usize;
        let value = get_string_field(state, 4, "byte")?
            .ok_or_else(|| lua_error_message("stage_attachment payload requires byte"))?;
        let byte = value
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| lua_error_message("stage_attachment byte must be non-empty"))?;
        let chunk = vec![byte; 8192.min(len)];
        let mut remaining = len;
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            writer.write_all(&chunk[..n]).map_err(lua_io)?;
            hasher.update(&chunk[..n]);
            remaining -= n;
        }
        len
    } else {
        let bytes = state.to_string(4)?;
        writer.write_all(bytes.as_bytes()).map_err(lua_io)?;
        hasher.update(bytes.as_bytes());
        bytes.len()
    };
    writer.flush().map_err(lua_io)?;
    let content_hash = *hasher.finalize().as_bytes();
    let content_hash_vec = content_hash.to_vec();
    let value = serde_json::json!({
        "relative_path": relative_path,
        "content_hash": content_hash_vec,
        "content_hash_hex": hex_bytes(&content_hash),
        "size": size,
        "source": {
            "kind": "staging_file",
            "relative_path": relative_path,
            "content_hash": content_hash_vec,
        },
    });
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_assert(state: &mut State) -> dellingr::Result<u8> {
    if state.to_boolean(1) {
        state.set_top(0);
        return Ok(0);
    }
    let message = if state.get_top() >= 2 {
        state.to_string(2)?
    } else {
        "assertion failed".to_string()
    };
    Err(lua_error_message(message))
}

fn lua_assert_eq(state: &mut State) -> dellingr::Result<u8> {
    if state.raw_equal(1, 2) {
        state.set_top(0);
        return Ok(0);
    }
    let actual = state.to_string(1)?;
    let expected = state.to_string(2)?;
    let prefix = if state.get_top() >= 3 {
        format!("{}: ", state.to_string(3)?)
    } else {
        String::new()
    };
    Err(lua_error_message(format!(
        "{prefix}expected {expected:?}, got {actual:?}"
    )))
}

fn lua_same_client(state: &mut State) -> dellingr::Result<u8> {
    let a = resource_id(state, 1)?;
    let b = resource_id(state, 2)?;
    let ctx = context(state)?;
    let same = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        match (guard.resources.get(&a), guard.resources.get(&b)) {
            (Some(HarnessResource::Client(a)), Some(HarnessResource::Client(b))) => {
                Arc::ptr_eq(a, b)
            }
            _ => false,
        }
    };
    state.set_top(0);
    state.push_boolean(same);
    Ok(1)
}

fn lua_env(state: &mut State) -> dellingr::Result<u8> {
    let name = state.to_string(1)?;
    state.set_top(0);
    match std::env::var(&name) {
        Ok(value) => state.push_string(&value),
        Err(_) => state.push_nil(),
    }
    Ok(1)
}

fn lua_join_url(state: &mut State) -> dellingr::Result<u8> {
    let base = state.to_string(1)?;
    let suffix = state.to_string(2)?;
    let url = join_url(&base, &suffix);
    state.set_top(0);
    state.push_string(&url);
    Ok(1)
}

fn lua_mock_requests(state: &mut State) -> dellingr::Result<u8> {
    let endpoint = state.to_string(1)?;
    let stable = mock_requests_stable_option(state)?;
    let url = mock_requests_url(&endpoint, stable);
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_get_json(handle, url, "mock_requests")?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_clear_mock_requests(state: &mut State) -> dellingr::Result<u8> {
    let endpoint = state.to_string(1)?;
    let url = mock_requests_url(&endpoint, false);
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    http_delete(handle, url, "clear_mock_requests")?;
    state.set_top(0);
    Ok(0)
}

fn lua_snapshot_state(state: &mut State) -> dellingr::Result<u8> {
    let endpoint = state.to_string(1)?;
    let url = snapshot_state_url(&endpoint);
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_get_json(handle, url, "snapshot_state")?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_latency(state: &mut State) -> dellingr::Result<u8> {
    let endpoint = state.to_string(1)?;
    let url = latency_url(&endpoint);
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_get_json(handle, url, "latency")?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_set_latency(state: &mut State) -> dellingr::Result<u8> {
    let endpoint = state.to_string(1)?;
    let request_body = latency_body_from_arg(state, 2)?;
    let url = latency_url(&endpoint);
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_request_json(
        handle,
        reqwest::Method::POST,
        url,
        Some(request_body),
        "set_latency",
    )?
    .ok_or_else(|| lua_error_message("set_latency returned an empty response body"))?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_request_count(state: &mut State) -> dellingr::Result<u8> {
    lua_request_count_impl(state, false, "request_count")
}

fn lua_request_count_prefix(state: &mut State) -> dellingr::Result<u8> {
    lua_request_count_impl(state, true, "request_count_prefix")
}

fn lua_request_count_impl(
    state: &mut State,
    prefix: bool,
    operation: &'static str,
) -> dellingr::Result<u8> {
    if state.typ(1) != LuaType::Table {
        return Err(lua_error_message(format!(
            "{operation} requires request table"
        )));
    }
    let protocol = state.to_string(2)?;
    let command = state.to_string(3)?;
    let requests_idx = 1;
    let len = state.table_len(requests_idx);
    let mut count = 0u64;
    for i in 1..=len {
        let top = state.get_top();
        state.push_number(i as f64);
        state.get_table(requests_idx)?;
        if state.typ(-1) == LuaType::Table {
            let request_idx = state.get_top() as isize;
            let request_protocol = get_string_field(state, request_idx, "protocol")?;
            let request_command = get_string_field(state, request_idx, "command")?;
            let command_matches = request_command.as_deref().is_some_and(|request_command| {
                if prefix {
                    request_command.starts_with(&command)
                } else {
                    request_command == command
                }
            });
            if request_protocol.as_deref() == Some(protocol.as_str()) && command_matches {
                count = count.saturating_add(1);
            }
        }
        state.set_top(top as isize);
    }
    state.set_top(0);
    state.push_number(count as f64);
    Ok(1)
}

fn lua_http_json(state: &mut State) -> dellingr::Result<u8> {
    if state.typ(1) != LuaType::Table {
        return Err(lua_error_message("http_json requires request table"));
    }
    let method = get_string_field(state, 1, "method")?
        .ok_or_else(|| lua_error_message("http_json requires request.method"))?;
    let url = get_string_field(state, 1, "url")?
        .ok_or_else(|| lua_error_message("http_json requires request.url"))?;
    let request_body = http_json_body_from_field(state, 1, "body")?;
    let method = parse_http_method(&method, "http_json")?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_request_json(handle, method, url, request_body, "http_json")?;
    state.set_top(0);
    match value {
        Some(value) => push_json(state, &value)?,
        None => state.push_nil(),
    }
    Ok(1)
}

fn lua_http_get(state: &mut State) -> dellingr::Result<u8> {
    let url = state.to_string(1)?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_get_json(handle, url, "http_get")?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_http_post_json(state: &mut State) -> dellingr::Result<u8> {
    let url = state.to_string(1)?;
    let request_body = state.to_string(2)?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let value = http_request_json(
        handle,
        reqwest::Method::POST,
        url,
        Some(request_body),
        "http_post_json",
    )?
    .ok_or_else(|| lua_error_message("http_post_json returned an empty response body"))?;
    state.set_top(0);
    push_json(state, &value)?;
    Ok(1)
}

fn lua_http_delete(state: &mut State) -> dellingr::Result<u8> {
    let url = state.to_string(1)?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    http_delete(handle, url, "http_delete")?;
    state.set_top(0);
    Ok(0)
}

fn lua_http(state: &mut State) -> dellingr::Result<u8> {
    if state.typ(1) != LuaType::Table {
        return Err(lua_error_message("http requires request table"));
    }
    let method = get_string_field(state, 1, "method")?
        .ok_or_else(|| lua_error_message("http requires request.method"))?;
    let url = get_string_field(state, 1, "url")?
        .ok_or_else(|| lua_error_message("http requires request.url"))?;
    let body = get_string_field(state, 1, "body")?;
    let content_type = get_string_field(state, 1, "content_type")?;
    let if_match = get_string_field(state, 1, "if_match")?;
    let method = parse_http_method(&method, "http")?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let response = http_request_text(
        handle,
        method,
        url,
        body,
        content_type,
        if_match,
        "http",
    )?;
    state.set_top(0);
    push_json(state, &response)?;
    Ok(1)
}

fn join_url(base: &str, suffix: &str) -> String {
    match (base.ends_with('/'), suffix.starts_with('/')) {
        (true, true) => format!("{}{}", base, &suffix[1..]),
        (true, false) | (false, true) => format!("{base}{suffix}"),
        (false, false) => format!("{base}/{suffix}"),
    }
}

fn mock_requests_stable_option(state: &mut State) -> dellingr::Result<bool> {
    if state.get_top() < 2 {
        return Ok(false);
    }
    match state.typ(2) {
        LuaType::Nil => Ok(false),
        LuaType::Boolean => Ok(state.to_boolean(2)),
        LuaType::Table => Ok(get_bool_field(state, 2, "stable")?.unwrap_or(false)),
        other => Err(lua_error_message(format!(
            "mock_requests options must be nil, boolean, or table, got {}",
            other.as_str()
        ))),
    }
}

fn mock_requests_url(endpoint: &str, stable: bool) -> String {
    if stable {
        join_url(endpoint, "test/requests?stable=true")
    } else {
        join_url(endpoint, "test/requests")
    }
}

fn snapshot_state_url(endpoint: &str) -> String {
    join_url(endpoint, "test/snapshot-state")
}

fn latency_url(endpoint: &str) -> String {
    join_url(endpoint, "test/latency")
}

fn latency_body_from_arg(state: &mut State, idx: isize) -> dellingr::Result<String> {
    if (state.get_top() as isize) < idx || state.typ(idx) == LuaType::Nil {
        return Ok("{}".to_string());
    }
    if state.typ(idx) != LuaType::Table {
        return Err(lua_error_message(format!(
            "set_latency options must be nil or table, got {}",
            state.typ(idx).as_str()
        )));
    }

    let mut body = serde_json::Map::new();
    if let Some(global_ms) = get_u64_field(state, idx, "global_ms", "set_latency.global_ms")? {
        body.insert(
            "global_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(global_ms)),
        );
    }
    if let Some(per_protocol) = latency_per_protocol_field(state, idx)? {
        body.insert("per_protocol".to_string(), per_protocol);
    }
    serde_json::to_string(&serde_json::Value::Object(body)).map_err(lua_json)
}

fn latency_per_protocol_field(
    state: &mut State,
    table_idx: isize,
) -> dellingr::Result<Option<serde_json::Value>> {
    let top = state.get_top();
    state.push_string("per_protocol");
    state.get_table(table_idx)?;
    let result = match state.typ(-1) {
        LuaType::Nil => Ok(None),
        LuaType::Table => {
            let per_idx = absolute_stack_idx(state, -1);
            let mut object = serde_json::Map::new();
            state.push_nil();
            loop {
                let has_next = match state.table_next(per_idx) {
                    Ok(has_next) => has_next,
                    Err(error) => {
                        state.set_top(top as isize);
                        return Err(error);
                    }
                };
                if !has_next {
                    break;
                }
                let key = match state.typ(-2) {
                    LuaType::String => state.to_string(-2),
                    other => Err(lua_error_message(format!(
                        "set_latency.per_protocol key must be string, got {}",
                        other.as_str()
                    ))),
                };
                let value = match key {
                    Ok(key) => match state.typ(-1) {
                        LuaType::Number => latency_u64(state.to_number(-1)?, &format!(
                            "set_latency.per_protocol.{key}"
                        ))
                        .map(|value| (key, value)),
                        other => Err(lua_error_message(format!(
                            "set_latency.per_protocol.{key} must be a non-negative integer, got {}",
                            other.as_str()
                        ))),
                    },
                    Err(error) => Err(error),
                };
                match value {
                    Ok((key, value)) => {
                        object.insert(
                            key,
                            serde_json::Value::Number(serde_json::Number::from(value)),
                        );
                    }
                    Err(error) => {
                        state.set_top(top as isize);
                        return Err(error);
                    }
                }
                state.pop(1);
            }
            Ok(Some(serde_json::Value::Object(object)))
        }
        other => Err(lua_error_message(format!(
            "set_latency.per_protocol must be table, got {}",
            other.as_str()
        ))),
    };
    state.set_top(top as isize);
    result
}

fn get_u64_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
    label: &str,
) -> dellingr::Result<Option<u64>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = match state.typ(-1) {
        LuaType::Nil => Ok(None),
        LuaType::Number => latency_u64(state.to_number(-1)?, label).map(Some),
        other => Err(lua_error_message(format!(
            "{label} must be a non-negative integer, got {}",
            other.as_str()
        ))),
    };
    state.set_top(top as isize);
    result
}

fn latency_u64(value: f64, label: &str) -> dellingr::Result<u64> {
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 {
        Ok(value as u64)
    } else {
        Err(lua_error_message(format!(
            "{label} must be a non-negative integer, got {value}"
        )))
    }
}

fn http_json_body_from_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Option<String>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = match state.typ(-1) {
        LuaType::Nil => Ok(None),
        LuaType::String => state.to_string(-1).map(Some),
        LuaType::Boolean | LuaType::Number | LuaType::Table => {
            let value = lua_value_to_json(state, -1)?;
            serde_json::to_string(&value)
                .map(Some)
                .map_err(lua_json)
        }
        other => Err(lua_error_message(format!(
            "http_json request.{key} must be nil, string, or JSON-like value, got {}",
            other.as_str()
        ))),
    };
    state.set_top(top as isize);
    result
}

fn parse_http_method(method: &str, operation: &'static str) -> dellingr::Result<reqwest::Method> {
    let method = method.to_ascii_uppercase();
    reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| {
        lua_error_message(format!(
            "{operation} invalid method {method:?}: {error}"
        ))
    })
}

fn http_get_json(
    handle: tokio::runtime::Handle,
    url: String,
    operation: &'static str,
) -> dellingr::Result<serde_json::Value> {
    http_request_json(handle, reqwest::Method::GET, url, None, operation)?
        .ok_or_else(|| lua_error_message(format!("{operation} returned an empty response body")))
}

fn http_request_json(
    handle: tokio::runtime::Handle,
    method: reqwest::Method,
    url: String,
    request_body: Option<String>,
    operation: &'static str,
) -> dellingr::Result<Option<serde_json::Value>> {
    let method_label = method.as_str().to_string();
    let body = handle
        .block_on(async move {
            let client = reqwest::Client::new();
            let mut request = client.request(method, &url);
            if let Some(request_body) = request_body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(request_body);
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("{operation} {method_label} {url}: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("{operation} {method_label} {url}: status {status}"));
            }
            response
                .text()
                .await
                .map_err(|e| format!("{operation} {method_label} {url} body: {e}"))
        })
        .map_err(lua_error_message)?;
    if body.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|e| lua_error_message(format!("{operation} JSON parse: {e}; body={body}")))
}

fn http_request_text(
    handle: tokio::runtime::Handle,
    method: reqwest::Method,
    url: String,
    request_body: Option<String>,
    content_type: Option<String>,
    if_match: Option<String>,
    operation: &'static str,
) -> dellingr::Result<serde_json::Value> {
    let method_label = method.as_str().to_string();
    let (status, body) = handle
        .block_on(async move {
            let client = reqwest::Client::new();
            let mut request = client.request(method, &url);
            if let Some(content_type) = content_type {
                request = request.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            if let Some(if_match) = if_match {
                request = request.header(reqwest::header::IF_MATCH, if_match);
            }
            if let Some(request_body) = request_body {
                request = request.body(request_body);
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("{operation} {method_label} {url}: {e}"))?;
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .map_err(|e| format!("{operation} {method_label} {url} body: {e}"))?;
            Ok::<_, String>((status, body))
        })
        .map_err(lua_error_message)?;
    Ok(serde_json::json!({
        "status": status,
        "ok": (200u16..300u16).contains(&status),
        "body": body,
    }))
}

fn http_delete(
    handle: tokio::runtime::Handle,
    url: String,
    operation: &'static str,
) -> dellingr::Result<()> {
    handle
        .block_on(async move {
            let client = reqwest::Client::new();
            let response = client
                .delete(&url)
                .send()
                .await
                .map_err(|e| format!("{operation} {url}: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("{operation} {url}: status {status}"));
            }
            Ok(())
        })
        .map_err(lua_error_message)
}

fn lua_expect_quiet(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let seconds = state.to_number(2)?;
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let event = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(HarnessResource::Events(rx)) = guard.resources.get_mut(&id) else {
            return Err(lua_error_message(format!("no events resource {id}")));
        };
        handle.block_on(async {
            tokio::time::timeout(duration_from_seconds(seconds), rx.recv()).await
        })
    };
    state.set_top(0);
    match event {
        Err(_) => {
            state.push_boolean(true);
            Ok(1)
        }
        Ok(None) => {
            state.push_boolean(true);
            Ok(1)
        }
        Ok(Some(event)) => {
            push_spawn_event(state, event, &ctx)?;
            state.push_boolean(false);
            state.insert(-2)?;
            Ok(2)
        }
    }
}

fn lua_client_request(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let method = state.to_string(2)?;
    let params = request_params_from_lua(state, &method, 3)?;
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(client.request_value_for_harness(params));
    state.set_top(0);
    push_result_pair(state, result)?;
    Ok(2)
}

fn lua_client_request_async(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let method = state.to_string(2)?;
    let params = request_params_from_lua(state, &method, 3)?;
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let task = handle.spawn(async move { client.request_value_for_harness(params).await });
    let request_id = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.insert(HarnessResource::Request(task))
    };
    state.set_top(0);
    push_request_table(state, request_id)?;
    Ok(1)
}

fn lua_client_notify(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let method = state.to_string(2)?;
    if state.get_top() >= 3 && state.typ(3) != LuaType::Nil {
        return Err(lua_error_message(
            "client:notify currently supports only params-less notifications",
        ));
    }
    let notification = ClientNotification::from_method_params(&method, &None)
        .map_err(lua_error_message)?;
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(client.send_notification(notification));
    state.set_top(0);
    match result {
        Ok(()) => {
            state.push_boolean(true);
            state.push_nil();
        }
        Err(error) => {
            state.push_boolean(false);
            push_client_error(state, &error)?;
        }
    }
    Ok(2)
}

fn lua_client_shutdown(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(client.shutdown());
    state.set_top(0);
    match result {
        Ok(()) => {
            state.push_boolean(true);
            state.push_nil();
        }
        Err(error) => {
            state.push_boolean(false);
            push_client_error(state, &error)?;
        }
    }
    Ok(2)
}

fn lua_client_child_pid(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let ctx = context(state)?;
    let pid = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.client(id)?.child_pid()
    };
    if let Some(pid) = pid
        && let Ok(ctx) = context(state)
    {
        ctx.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_pid = Some(pid);
    }
    state.set_top(0);
    match pid {
        Some(pid) => state.push_number(pid as f64),
        None => state.push_nil(),
    }
    Ok(1)
}

fn lua_client_current_generation(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let ctx = context(state)?;
    let generation = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.client(id)?.current_generation()
    };
    state.set_top(0);
    state.push_number(generation as f64);
    Ok(1)
}

fn lua_client_notification_should_dispatch(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    if state.get_top() < 2 || state.typ(2) != LuaType::Table {
        return Err(lua_error_message(
            "notification_should_dispatch requires notification table",
        ));
    }
    let method_name = get_string_field(state, 2, "method")?
        .or_else(|| get_string_field(state, 2, "type").ok().flatten())
        .unwrap_or_else(|| "notification".to_string());
    let service_generation =
        get_number_field(state, 2, "service_generation")?.map(|value| value as u32);
    let ctx = context(state)?;
    let current_generation = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.client(id)?.current_generation()
    };
    let dispatch = crate::service_client::notification_generation_should_dispatch(
        service_generation,
        current_generation,
        &method_name,
    );
    state.set_top(0);
    state.push_boolean(dispatch);
    Ok(1)
}

fn lua_client_set_respawn_args(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let extra_args = read_extra_args(state, 2)?;
    let ctx = context(state)?;
    let updated = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .client(id)?
            .set_respawn_extra_args_for_harness(extra_args)
    };
    state.set_top(0);
    state.push_boolean(updated);
    Ok(1)
}

fn lua_client_notifications(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let ctx = context(state)?;
    let notifications = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.client(id)?.notifications()
    };
    let queue_id = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.insert(HarnessResource::Notifications(notifications))
    };
    state.set_top(0);
    push_notifications_table(state, queue_id)?;
    Ok(1)
}

fn lua_client_start_sync(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    if state.get_top() < 2 {
        return Err(lua_error_message("start_sync requires account_id"));
    }
    let account_id = if state.typ(2) == LuaType::Table {
        get_string_field(state, 2, "account_id")?
            .ok_or_else(|| lua_error_message("start_sync requires params.account_id"))?
    } else {
        state.to_string(2)?
    };
    let seconds = if state.get_top() >= 3 {
        state.to_number(3)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(async {
        tokio::time::timeout(
            duration_from_seconds(seconds),
            client.start_sync(account_id),
        )
        .await
    });
    state.set_top(0);
    match result {
        Ok(Ok(sync_result)) => {
            push_sync_result_table(state, &sync_result)?;
            state.push_nil();
        }
        Ok(Err(error)) => {
            state.push_nil();
            push_client_error(state, &error)?;
        }
        Err(_) => {
            state.push_nil();
            push_json(
                state,
                &serde_json::json!({
                    "kind": "Timeout",
                    "detail": format!("sync did not resolve within {seconds}s"),
                }),
            )?;
        }
    }
    Ok(2)
}

fn lua_client_start_calendar_sync(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    if state.get_top() < 2 {
        return Err(lua_error_message("start_calendar_sync requires account_id"));
    }
    let account_id = if state.typ(2) == LuaType::Table {
        get_string_field(state, 2, "account_id")?
            .ok_or_else(|| lua_error_message("start_calendar_sync requires params.account_id"))?
    } else {
        state.to_string(2)?
    };
    let seconds = if state.get_top() >= 3 {
        state.to_number(3)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(async {
        tokio::time::timeout(
            duration_from_seconds(seconds),
            client.start_calendar_sync(account_id),
        )
        .await
    });
    state.set_top(0);
    match result {
        Ok(Ok(sync_result)) => {
            push_calendar_sync_result_table(state, &sync_result)?;
            state.push_nil();
        }
        Ok(Err(error)) => {
            state.push_nil();
            push_client_error(state, &error)?;
        }
        Err(_) => {
            state.push_nil();
            push_json(
                state,
                &serde_json::json!({
                    "kind": "Timeout",
                    "detail": format!("calendar sync did not resolve within {seconds}s"),
                }),
            )?;
        }
    }
    Ok(2)
}

fn lua_client_execute_calendar_plan(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    if state.get_top() < 2 {
        return Err(lua_error_message("execute_calendar_plan requires plan table"));
    }
    let plan = parse_calendar_action_plan(state, 2)?;
    let seconds = if state.get_top() >= 3 {
        state.to_number(3)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let (handle, client) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        (guard.handle.clone(), guard.client(id)?)
    };
    let result = handle.block_on(async {
        tokio::time::timeout(duration_from_seconds(seconds), async move {
            let ack = client.execute_calendar_plan(plan).await?;
            client.subscribe_or_consume_calendar_action(ack.plan_id).await
        })
        .await
    });
    state.set_top(0);
    match result {
        Ok(Ok(completed)) => {
            push_json(
                state,
                &serde_json::to_value(&completed).map_err(lua_json)?,
            )?;
            state.push_nil();
        }
        Ok(Err(error)) => {
            state.push_nil();
            push_client_error(state, &error)?;
        }
        Err(_) => {
            state.push_nil();
            push_json(
                state,
                &serde_json::json!({
                    "kind": "Timeout",
                    "detail": format!("calendar action did not resolve within {seconds}s"),
                }),
            )?;
        }
    }
    Ok(2)
}

fn lua_client_drop(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    context(state)?
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(id);
    state.set_top(0);
    Ok(0)
}

fn lua_notifications_recv(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let seconds = if state.get_top() >= 2 {
        state.to_number(2)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let (handle, queue) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(HarnessResource::Notifications(queue)) = guard.resources.get(&id) else {
            return Err(lua_error_message(format!("no notification queue resource {id}")));
        };
        (guard.handle.clone(), Arc::clone(queue))
    };
    let result = handle.block_on(async {
        tokio::time::timeout(duration_from_seconds(seconds), queue.recv()).await
    });
    state.set_top(0);
    match result {
        Ok(Some(notification)) => push_notification(state, &notification)?,
        Ok(None) | Err(_) => state.push_nil(),
    }
    Ok(1)
}

fn lua_notifications_drain_for(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let seconds = state.to_number(2)?;
    let ctx = context(state)?;
    let (handle, queue) = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(HarnessResource::Notifications(queue)) = guard.resources.get(&id) else {
            return Err(lua_error_message(format!("no notification queue resource {id}")));
        };
        (guard.handle.clone(), Arc::clone(queue))
    };
    let notifications = handle.block_on(async move {
        let deadline = Instant::now() + duration_from_seconds(seconds);
        let mut notifications = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, queue.recv()).await {
                Ok(Some(notification)) => notifications.push(notification),
                Ok(None) | Err(_) => break,
            }
        }
        notifications
    });
    state.set_top(0);
    state.new_table();
    let idx = state.get_top() as isize;
    for (offset, notification) in notifications.iter().enumerate() {
        state.push_number((offset + 1) as f64);
        push_notification(state, notification)?;
        state.set_table_raw(idx)?;
    }
    Ok(1)
}

fn lua_events_next(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let seconds = if state.get_top() >= 2 {
        state.to_number(2)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let handle = {
        let guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        guard.handle.clone()
    };
    let event = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(HarnessResource::Events(rx)) = guard.resources.get_mut(&id) else {
            return Err(lua_error_message(format!("no events resource {id}")));
        };
        handle.block_on(async {
            tokio::time::timeout(duration_from_seconds(seconds), rx.recv()).await
        })
    };
    state.set_top(0);
    match event {
        Ok(Some(event)) => {
            push_spawn_event(state, event, &ctx)?;
            Ok(1)
        }
        Ok(None) => {
            state.push_nil();
            Ok(1)
        }
        Err(_) => Err(lua_error_message(format!(
            "event {id} did not arrive within {seconds}s"
        ))),
    }
}

fn lua_request_await(state: &mut State) -> dellingr::Result<u8> {
    let id = resource_id(state, 1)?;
    let seconds = if state.get_top() >= 2 {
        state.to_number(2)?
    } else {
        30.0
    };
    let ctx = context(state)?;
    let (handle, task) = {
        let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(HarnessResource::Request(_)) = guard.resources.get(&id) else {
            return Err(lua_error_message(format!("no request resource {id}")));
        };
        let Some(HarnessResource::Request(task)) = guard.resources.remove(&id) else {
            return Err(lua_error_message(format!("no request resource {id}")));
        };
        (guard.handle.clone(), task)
    };
    let result = handle.block_on(async {
        tokio::time::timeout(duration_from_seconds(seconds), task).await
    });
    state.set_top(0);
    match result {
        Ok(Ok(request_result)) => push_result_pair(state, request_result)?,
        Ok(Err(error)) => {
            state.push_nil();
            push_json(
                state,
                &serde_json::json!({
                    "kind": "Join",
                    "detail": error.to_string(),
                }),
            )?;
        }
        Err(_) => {
            state.push_nil();
            push_json(
                state,
                &serde_json::json!({
                    "kind": "Timeout",
                    "detail": format!("request did not resolve within {seconds}s"),
                }),
            )?;
        }
    }
    Ok(2)
}

fn push_result_pair(
    state: &mut State,
    result: Result<serde_json::Value, ClientError>,
) -> dellingr::Result<()> {
    match result {
        Ok(value) => {
            push_json(state, &value)?;
            state.push_nil();
        }
        Err(error) => {
            state.push_nil();
            push_client_error(state, &error)?;
        }
    }
    Ok(())
}

fn push_spawn_event(
    state: &mut State,
    event: SpawnEvent,
    ctx: &Arc<Mutex<HarnessContext>>,
) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    match event {
        SpawnEvent::ChildSpawned(client) => {
            let (id, pid) = {
                let mut guard = ctx.lock().unwrap_or_else(PoisonError::into_inner);
                let pid = client.child_pid();
                guard.last_pid = pid;
                (guard.insert(HarnessResource::Client(client)), pid)
            };
            set_field_string(state, idx, "type", "ChildSpawned")?;
            push_client_table(state, id)?;
            set_pushed_field(state, idx, "client")?;
            if let Some(pid) = pid {
                set_field_number(state, idx, "pid", pid as f64)?;
            }
        }
        SpawnEvent::BootReady(response) => {
            set_field_string(state, idx, "type", "BootReady")?;
            push_json(state, &serde_json::to_value(response).map_err(lua_json)?)?;
            set_pushed_field(state, idx, "response")?;
        }
        SpawnEvent::Terminal(error) => {
            set_field_string(state, idx, "type", "Terminal")?;
            push_client_error(state, &error)?;
            set_pushed_field(state, idx, "error")?;
        }
        SpawnEvent::HealthChanged(health) => {
            set_field_string(state, idx, "type", "HealthChanged")?;
            set_field_string(state, idx, "health", &format!("{health:?}"))?;
        }
    }
    let event_json = table_summary_json(state, idx)?;
    ctx.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .record_event(&event_json);
    Ok(())
}

fn push_client_error(state: &mut State, error: &ClientError) -> dellingr::Result<()> {
    let value = match error {
        ClientError::Io(error) => serde_json::json!({
            "kind": "Io",
            "detail": error.to_string(),
        }),
        ClientError::Service(service_api::ServiceError::BootFailure { code }) => {
            serde_json::json!({
                "kind": "Service",
                "service_kind": "BootFailure",
                "boot_code": boot_code_name(*code),
                "boot_code_num": code.as_i32(),
            })
        }
        ClientError::Service(error) => serde_json::json!({
            "kind": "Service",
            "detail": error.to_string(),
        }),
        ClientError::Timeout => serde_json::json!({ "kind": "Timeout" }),
        ClientError::ServiceCrashed => serde_json::json!({ "kind": "ServiceCrashed" }),
        ClientError::NotConnected => serde_json::json!({ "kind": "NotConnected" }),
        ClientError::VersionMismatch { ui, service } => serde_json::json!({
            "kind": "VersionMismatch",
            "ui": ui,
            "service": service,
        }),
        ClientError::BootFailure { classification } => match classification {
            BootClassification::BootFailure { code } => serde_json::json!({
                "kind": "BootFailure",
                "classification": "BootFailure",
                "boot_code": boot_code_name(*code),
                "boot_code_num": code.as_i32(),
            }),
            BootClassification::UnexpectedExit { code } => serde_json::json!({
                "kind": "BootFailure",
                "classification": "UnexpectedExit",
                "exit_code": code,
            }),
        },
        ClientError::SchemaVersionChanged { was, now } => serde_json::json!({
            "kind": "SchemaVersionChanged",
            "was": was,
            "now": now,
        }),
        ClientError::SchemaBaselineMissing => {
            serde_json::json!({ "kind": "SchemaBaselineMissing" })
        }
        ClientError::Deserialize(error) => serde_json::json!({
            "kind": "Deserialize",
            "detail": error.to_string(),
        }),
    };
    push_json(state, &value)
}

fn push_client_table(state: &mut State, id: u64) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "__harness_type", "client")?;
    set_field_number(state, idx, "__harness_id", id as f64)?;
    set_field_fn(state, idx, "request", lua_client_request)?;
    set_field_fn(state, idx, "request_async", lua_client_request_async)?;
    set_field_fn(state, idx, "notify", lua_client_notify)?;
    set_field_fn(state, idx, "shutdown", lua_client_shutdown)?;
    set_field_fn(state, idx, "child_pid", lua_client_child_pid)?;
    set_field_fn(state, idx, "current_generation", lua_client_current_generation)?;
    set_field_fn(
        state,
        idx,
        "notification_should_dispatch",
        lua_client_notification_should_dispatch,
    )?;
    set_field_fn(state, idx, "set_respawn_args", lua_client_set_respawn_args)?;
    set_field_fn(state, idx, "notifications", lua_client_notifications)?;
    set_field_fn(state, idx, "start_sync", lua_client_start_sync)?;
    set_field_fn(
        state,
        idx,
        "start_calendar_sync",
        lua_client_start_calendar_sync,
    )?;
    set_field_fn(
        state,
        idx,
        "execute_calendar_plan",
        lua_client_execute_calendar_plan,
    )?;
    set_field_fn(state, idx, "drop", lua_client_drop)?;
    Ok(())
}

fn push_events_table(state: &mut State, id: u64) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "__harness_type", "events")?;
    set_field_number(state, idx, "__harness_id", id as f64)?;
    set_field_fn(state, idx, "next", lua_events_next)?;
    Ok(())
}

fn push_request_table(state: &mut State, id: u64) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "__harness_type", "request")?;
    set_field_number(state, idx, "__harness_id", id as f64)?;
    set_field_fn(state, idx, "await", lua_request_await)?;
    Ok(())
}

fn push_notifications_table(state: &mut State, id: u64) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "__harness_type", "notifications")?;
    set_field_number(state, idx, "__harness_id", id as f64)?;
    set_field_fn(state, idx, "recv", lua_notifications_recv)?;
    set_field_fn(state, idx, "drain_for", lua_notifications_drain_for)?;
    Ok(())
}

fn push_notification(state: &mut State, notification: &Notification) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "method", notification.method_name())?;
    match notification {
        Notification::BootProgress(progress) => {
            set_field_string(state, idx, "type", "BootProgress")?;
            set_field_string(
                state,
                idx,
                "phase_kind",
                boot_phase_kind_name(progress.phase.coalesce_discriminant()),
            )?;
            set_field_string(state, idx, "phase", &format!("{:?}", progress.phase))?;
            set_field_number(state, idx, "service_generation", progress.service_generation as f64)?;
        }
        Notification::OperationOutcome(outcome) => {
            set_field_string(state, idx, "type", "OperationOutcome")?;
            set_field_string(state, idx, "plan_id", &outcome.plan_id.to_string())?;
            set_field_number(state, idx, "operation_id", outcome.operation_id.0 as f64)?;
            set_field_number(
                state,
                idx,
                "service_generation",
                outcome.service_generation as f64,
            )?;
        }
        Notification::ActionCompleted(completed) => {
            set_field_string(state, idx, "type", "ActionCompleted")?;
            set_field_string(state, idx, "plan_id", &completed.plan_id.to_string())?;
            set_field_number(
                state,
                idx,
                "summary_total",
                completed.summary.total as f64,
            )?;
            set_field_number(
                state,
                idx,
                "summary_local_only",
                completed.summary.local_only as f64,
            )?;
            set_field_number(
                state,
                idx,
                "summary_remote_succeeded",
                completed.summary.remote_succeeded as f64,
            )?;
            set_field_number(
                state,
                idx,
                "summary_remote_failed",
                completed.summary.remote_failed as f64,
            )?;
            set_field_number(
                state,
                idx,
                "summary_conflicts",
                completed.summary.conflicts as f64,
            )?;
            set_field_number(
                state,
                idx,
                "service_generation",
                completed.service_generation as f64,
            )?;
        }
        Notification::SyncCompleted(completed) => {
            set_field_string(state, idx, "type", "SyncCompleted")?;
            set_field_string(state, idx, "account_id", &completed.account_id)?;
            set_field_string(state, idx, "run_id", &completed.run_id.to_string())?;
            set_field_string(
                state,
                idx,
                "result",
                sync_result_name(&completed.result),
            )?;
            set_field_number(
                state,
                idx,
                "service_generation",
                completed.service_generation as f64,
            )?;
        }
        Notification::IndexRebuildProgress(progress) => {
            set_field_string(state, idx, "type", "IndexRebuildProgress")?;
            set_field_string(state, idx, "rebuild_id", &progress.rebuild_id)?;
            set_field_number(state, idx, "processed", progress.processed as f64)?;
            set_field_number(state, idx, "total", progress.total as f64)?;
            set_field_number(
                state,
                idx,
                "service_generation",
                progress.service_generation as f64,
            )?;
        }
        Notification::IndexRebuildCompleted(completed) => {
            set_field_string(state, idx, "type", "IndexRebuildCompleted")?;
            set_field_string(state, idx, "rebuild_id", &completed.rebuild_id)?;
            set_field_number(
                state,
                idx,
                "service_generation",
                completed.service_generation as f64,
            )?;
        }
        Notification::PrefetchProgress(progress) => {
            set_field_string(state, idx, "type", "PrefetchProgress")?;
            set_field_number(state, idx, "remaining", progress.remaining as f64)?;
            set_field_number(
                state,
                idx,
                "fetched_in_session",
                progress.fetched_in_session as f64,
            )?;
            set_field_number(
                state,
                idx,
                "service_generation",
                progress.service_generation as f64,
            )?;
        }
        Notification::PrefetchCompleted(completed) => {
            set_field_string(state, idx, "type", "PrefetchCompleted")?;
            set_field_number(state, idx, "fetched", completed.fetched as f64)?;
            set_field_number(state, idx, "skipped", completed.skipped as f64)?;
            set_field_number(state, idx, "failed", completed.failed as f64)?;
            set_field_number(
                state,
                idx,
                "service_generation",
                completed.service_generation as f64,
            )?;
        }
        other => {
            set_field_string(state, idx, "type", other.method_name())?;
        }
    }
    push_json(state, &serde_json::to_value(notification).map_err(lua_json)?)?;
    set_pushed_field(state, idx, "raw")?;
    Ok(())
}

fn sync_result_name(result: &service_api::SyncResult) -> &'static str {
    match result {
        service_api::SyncResult::Completed => "completed",
        service_api::SyncResult::Cancelled => "cancelled",
        service_api::SyncResult::Failed(_) => "failed",
    }
}

fn push_sync_result_table(
    state: &mut State,
    result: &service_api::SyncResult,
) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "result", sync_result_name(result))?;
    if let service_api::SyncResult::Failed(error) = result {
        set_field_string(state, idx, "error", error)?;
    }
    Ok(())
}

fn calendar_sync_result_name(result: &service_api::CalendarSyncResult) -> &'static str {
    match result {
        service_api::CalendarSyncResult::Completed => "completed",
        service_api::CalendarSyncResult::Cancelled => "cancelled",
        service_api::CalendarSyncResult::Failed(_) => "failed",
    }
}

fn push_calendar_sync_result_table(
    state: &mut State,
    result: &service_api::CalendarSyncResult,
) -> dellingr::Result<()> {
    state.new_table();
    let idx = state.get_top() as isize;
    set_field_string(state, idx, "result", calendar_sync_result_name(result))?;
    if let service_api::CalendarSyncResult::Failed(error) = result {
        set_field_string(state, idx, "error", error)?;
    }
    Ok(())
}

fn request_params_from_lua(
    state: &mut State,
    method: &str,
    params_idx: isize,
) -> dellingr::Result<RequestParams> {
    match method {
        "HealthPing" | "health.ping" => Ok(RequestParams::HealthPing),
        "Shutdown" | "shutdown" => Ok(RequestParams::Shutdown),
        "BootReady" | "boot.ready" => Ok(RequestParams::BootReady),
        "ActionExecutePlan" | "action.execute_plan" => {
            let plan = parse_action_plan(state, params_idx)?;
            Ok(RequestParams::ActionExecutePlan { plan })
        }
        "CalActionExecutePlan" | "cal_action.execute_plan" => {
            let plan = parse_calendar_action_plan(state, params_idx)?;
            Ok(RequestParams::CalActionExecutePlan { plan })
        }
        "ActionJobStatus" | "action.job_status" => {
            let plan_id = parse_plan_id_request(state, params_idx, "ActionJobStatus")?;
            Ok(RequestParams::ActionJobStatus { plan_id })
        }
        "ActionSend" | "action.send" => {
            let request = parse_send_request(state, params_idx)?;
            Ok(RequestParams::ActionSend {
                request: Box::new(request),
            })
        }
        "ActionMarkChatRead" | "action.mark_chat_read" => {
            let chat_email = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) == LuaType::Table
            {
                get_string_field(state, params_idx, "chat_email")?
                    .ok_or_else(|| lua_error_message("ActionMarkChatRead requires chat_email"))?
            } else if state.get_top() >= params_idx as usize {
                state.to_string(params_idx)?
            } else {
                return Err(lua_error_message("ActionMarkChatRead requires chat_email"));
            };
            Ok(RequestParams::ActionMarkChatRead { chat_email })
        }
        "AccountDelete" | "account.delete" => {
            let account_id = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) == LuaType::Table
            {
                get_string_field(state, params_idx, "account_id")?
                    .ok_or_else(|| lua_error_message("AccountDelete requires account_id"))?
            } else if state.get_top() >= params_idx as usize {
                state.to_string(params_idx)?
            } else {
                return Err(lua_error_message("AccountDelete requires account_id"));
            };
            Ok(RequestParams::AccountDelete {
                params: AccountDeleteParams { account_id },
            })
        }
        "OauthExchangeCode" | "oauth.exchange_code" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("OauthExchangeCode requires params table"));
            }
            let provider_id = get_string_field(state, params_idx, "provider_id")?
                .ok_or_else(|| lua_error_message("OauthExchangeCode requires params.provider_id"))?;
            let token_url = get_string_field(state, params_idx, "token_url")?
                .ok_or_else(|| lua_error_message("OauthExchangeCode requires params.token_url"))?;
            let client_id = get_string_field(state, params_idx, "client_id")?
                .ok_or_else(|| lua_error_message("OauthExchangeCode requires params.client_id"))?;
            let redirect_uri = get_string_field(state, params_idx, "redirect_uri")?
                .ok_or_else(|| lua_error_message("OauthExchangeCode requires params.redirect_uri"))?;
            let code = get_string_field(state, params_idx, "code")?
                .ok_or_else(|| lua_error_message("OauthExchangeCode requires params.code"))?;
            Ok(RequestParams::OauthExchangeCode {
                params: Box::new(OauthExchangeCodeParams {
                    provider_id,
                    token_url,
                    scopes: get_string_array_field(state, params_idx, "scopes")?,
                    user_info_url: get_string_field(state, params_idx, "user_info_url")?,
                    use_pkce: get_bool_field(state, params_idx, "use_pkce")?.unwrap_or(false),
                    client_id,
                    client_secret: get_string_field(state, params_idx, "client_secret")?
                        .map(RedactedString::new),
                    redirect_uri,
                    code: RedactedString::new(code),
                    code_verifier: get_string_field(state, params_idx, "code_verifier")?,
                    reauth_account_id: get_string_field(state, params_idx, "reauth_account_id")?,
                }),
            })
        }
        "SettingsSet" | "settings.set" => Ok(RequestParams::SettingsSet {
            params: parse_settings_set_params(state, params_idx)?,
        }),
        "ContactsContactSave" | "contacts.contact_save" => {
            Ok(RequestParams::ContactsContactSave {
                params: parse_contact_save_params(state, params_idx)?,
            })
        }
        "ContactsContactSaveWithWriteback" | "contacts.contact_save_with_writeback" => {
            Ok(RequestParams::ContactsContactSaveWithWriteback {
                params: parse_contact_save_params(state, params_idx)?,
            })
        }
        "ContactsContactDelete" | "contacts.contact_delete" => {
            let id = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) == LuaType::Table
            {
                get_string_field(state, params_idx, "id")?
                    .ok_or_else(|| lua_error_message("ContactsContactDelete requires params.id"))?
            } else if state.get_top() >= params_idx as usize {
                state.to_string(params_idx)?
            } else {
                return Err(lua_error_message("ContactsContactDelete requires id"));
            };
            Ok(RequestParams::ContactsContactDelete {
                params: ContactDeleteParams { id },
            })
        }
        "ReadBootstrapSnapshots" | "internal.read_bootstrap_snapshots" => {
            Ok(RequestParams::ReadBootstrapSnapshots {
                params: ReadBootstrapSnapshotsParams::default(),
            })
        }
        "ExtractStatus" | "extract.status" => Ok(RequestParams::ExtractStatus {
            params: ExtractStatusParams::default(),
        }),
        "AttachmentFetch" | "attachment.fetch" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("AttachmentFetch requires params table"));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?
                .ok_or_else(|| lua_error_message("AttachmentFetch requires params.account_id"))?;
            let message_id = get_string_field(state, params_idx, "message_id")?
                .ok_or_else(|| lua_error_message("AttachmentFetch requires params.message_id"))?;
            let attachment_id = get_string_field(state, params_idx, "attachment_id")?
                .ok_or_else(|| {
                    lua_error_message("AttachmentFetch requires params.attachment_id")
                })?;
            Ok(RequestParams::AttachmentFetch {
                params: AttachmentFetchParams {
                    account_id,
                    message_id,
                    attachment_id,
                },
            })
        }
        "IndexRebuild" | "index.rebuild" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("IndexRebuild requires params table"));
            }
            let policy = get_string_field(state, params_idx, "policy")?
                .ok_or_else(|| lua_error_message("IndexRebuild requires params.policy"))?;
            let policy = match policy.as_str() {
                "wipe" => RebuildPolicy::Wipe,
                "preserve_existing" => RebuildPolicy::PreserveExisting,
                other => {
                    return Err(lua_error_message(format!(
                        "IndexRebuild unknown policy {other:?}"
                    )));
                }
            };
            let force = get_bool_field(state, params_idx, "force")?.unwrap_or(false);
            Ok(RequestParams::IndexRebuild {
                params: IndexRebuildParams { policy, force },
            })
        }
        "TestSlow" | "test.slow" => {
            let millis = if state.get_top() >= params_idx as usize {
                match state.typ(params_idx) {
                    LuaType::Number => state.to_number(params_idx)? as u64,
                    LuaType::Table => get_number_field(state, params_idx, "millis")?
                        .ok_or_else(|| lua_error_message("TestSlow requires params.millis"))?
                        as u64,
                    _ => return Err(lua_error_message("TestSlow params must be table or number")),
                }
            } else {
                return Err(lua_error_message("TestSlow requires millis"));
            };
            Ok(RequestParams::TestSlow { millis })
        }
        "TestPrintln" | "test.println" => {
            let message = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) == LuaType::Table
            {
                get_string_field(state, params_idx, "message")?
                    .ok_or_else(|| lua_error_message("TestPrintln requires params.message"))?
            } else if state.get_top() >= params_idx as usize {
                state.to_string(params_idx)?
            } else {
                return Err(lua_error_message("TestPrintln requires message"));
            };
            Ok(RequestParams::TestPrintln { message })
        }
"AccountUpdate" | "account.update" => {
            if state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("AccountUpdate params must be table"));
            }
            let id = get_string_field(state, params_idx, "id")?
                .ok_or_else(|| lua_error_message("AccountUpdate requires params.id"))?;
            let params = service_api::AccountUpdateParams {
                id,
                account_name:              get_string_field(state, params_idx, "account_name")?,
                display_name:              get_string_field(state, params_idx, "display_name")?,
                account_color:             get_string_field(state, params_idx, "account_color")?,
                caldav_url:                get_string_field(state, params_idx, "caldav_url")?,
                caldav_username:           get_string_field(state, params_idx, "caldav_username")?,
                caldav_password:           get_string_field(state, params_idx, "caldav_password")?,
                cache_attachments_enabled: get_bool_field(
                    state,
                    params_idx,
                    "cache_attachments_enabled",
                )?,
            };
            Ok(RequestParams::AccountUpdate { params })
        }
        "AttachmentCacheSize" | "attachment.cache_size" => {
            Ok(RequestParams::AttachmentCacheSize {
                params: service_api::AttachmentCacheSizeParams::default(),
            })
        }
        "TestSeedAccount" | "test.seed_account" => {
            let params = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) != LuaType::Nil
            {
                if state.typ(params_idx) != LuaType::Table {
                    return Err(lua_error_message("TestSeedAccount params must be table"));
                }
                TestSeedAccountParams {
                    email: get_string_field(state, params_idx, "email")?,
                    display_name: get_string_field(state, params_idx, "display_name")?,
                    account_name: get_string_field(state, params_idx, "account_name")?,
                    provider: get_string_field(state, params_idx, "provider")?,
                    caldav_url: get_string_field(state, params_idx, "caldav_url")?,
                    caldav_username: get_string_field(state, params_idx, "caldav_username")?,
                    caldav_password: get_string_field(state, params_idx, "caldav_password")?,
                    auth_method: get_string_field(state, params_idx, "auth_method")?,
                    access_token: get_string_field(state, params_idx, "access_token")?,
                    refresh_token: get_string_field(state, params_idx, "refresh_token")?,
                    token_expires_at: get_number_field(state, params_idx, "token_expires_at")?
                        .map(|value| value as i64),
                    oauth_provider: get_string_field(state, params_idx, "oauth_provider")?,
                    oauth_client_id: get_string_field(state, params_idx, "oauth_client_id")?,
                    oauth_token_url: get_string_field(state, params_idx, "oauth_token_url")?,
                }
            } else {
                TestSeedAccountParams::default()
            };
            Ok(RequestParams::TestSeedAccount { params })
        }
        "TestCounterRead" | "test.counter_read" => {
            let counter = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) == LuaType::Table
            {
                get_string_field(state, params_idx, "counter")?
                    .ok_or_else(|| lua_error_message("TestCounterRead requires params.counter"))?
            } else if state.get_top() >= params_idx as usize {
                state.to_string(params_idx)?
            } else {
                return Err(lua_error_message("TestCounterRead requires counter"));
            };
            Ok(RequestParams::TestCounterRead { counter })
        }
        "TestCrashAfterNWrites" | "test.crash_after_n_writes" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message(
                    "TestCrashAfterNWrites requires params table",
                ));
            }
            let kind = get_string_field(state, params_idx, "kind")?
                .ok_or_else(|| lua_error_message("TestCrashAfterNWrites requires params.kind"))?;
            let n = get_number_field(state, params_idx, "n")?
                .ok_or_else(|| lua_error_message("TestCrashAfterNWrites requires params.n"))?;
            Ok(RequestParams::TestCrashAfterNWrites {
                params: TestCrashAfterNWritesParams { kind, n: n as u64 },
            })
        }
        "TestSeedThread" | "test.seed_thread" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("TestSeedThread requires params table"));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?
                .ok_or_else(|| lua_error_message("TestSeedThread requires params.account_id"))?;
            Ok(RequestParams::TestSeedThread {
                params: TestSeedThreadParams {
                    account_id,
                    thread_id: get_string_field(state, params_idx, "thread_id")?,
                    message_id: get_string_field(state, params_idx, "message_id")?,
                    subject: get_string_field(state, params_idx, "subject")?,
                    label_ids: get_string_array_field(state, params_idx, "label_ids")?,
                    is_read: get_bool_field(state, params_idx, "is_read")?.unwrap_or(false),
                    is_starred: get_bool_field(state, params_idx, "is_starred")?.unwrap_or(false),
                    is_pinned: get_bool_field(state, params_idx, "is_pinned")?.unwrap_or(false),
                    is_muted: get_bool_field(state, params_idx, "is_muted")?.unwrap_or(false),
                    is_chat_thread: get_bool_field(state, params_idx, "is_chat_thread")?
                        .unwrap_or(false),
                    chat_email: get_string_field(state, params_idx, "chat_email")?,
                    body_text: get_string_field(state, params_idx, "body_text")?,
                    body_html: get_string_field(state, params_idx, "body_html")?,
                },
            })
        }
        "TestSeedCachedAttachment" | "test.seed_cached_attachment" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message(
                    "TestSeedCachedAttachment requires params table",
                ));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?.ok_or_else(|| {
                lua_error_message("TestSeedCachedAttachment requires params.account_id")
            })?;
            let message_id = get_string_field(state, params_idx, "message_id")?.ok_or_else(|| {
                lua_error_message("TestSeedCachedAttachment requires params.message_id")
            })?;
            let content = get_string_field(state, params_idx, "content")?.ok_or_else(|| {
                lua_error_message("TestSeedCachedAttachment requires params.content")
            })?;
            Ok(RequestParams::TestSeedCachedAttachment {
                params: TestSeedCachedAttachmentParams {
                    account_id,
                    message_id,
                    attachment_id: get_string_field(state, params_idx, "attachment_id")?,
                    filename: get_string_field(state, params_idx, "filename")?,
                    mime_type: get_string_field(state, params_idx, "mime_type")?
                        .or_else(|| get_string_field(state, params_idx, "mime").ok().flatten()),
                    content,
                },
            })
        }
        "TestSeedRemoteAttachment" | "test.seed_remote_attachment" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message(
                    "TestSeedRemoteAttachment requires params table",
                ));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?.ok_or_else(|| {
                lua_error_message("TestSeedRemoteAttachment requires params.account_id")
            })?;
            let message_id = get_string_field(state, params_idx, "message_id")?.ok_or_else(|| {
                lua_error_message("TestSeedRemoteAttachment requires params.message_id")
            })?;
            let content_base64 =
                get_string_field(state, params_idx, "content_base64")?.ok_or_else(|| {
                    lua_error_message(
                        "TestSeedRemoteAttachment requires params.content_base64",
                    )
                })?;
            Ok(RequestParams::TestSeedRemoteAttachment {
                params: TestSeedRemoteAttachmentParams {
                    account_id,
                    message_id,
                    attachment_id: get_string_field(state, params_idx, "attachment_id")?,
                    filename: get_string_field(state, params_idx, "filename")?,
                    mime_type: get_string_field(state, params_idx, "mime_type")?
                        .or_else(|| get_string_field(state, params_idx, "mime").ok().flatten()),
                    content_base64,
                },
            })
        }
        "TestRemoveCachedAttachmentBytes" | "test.remove_cached_attachment_bytes" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message(
                    "TestRemoveCachedAttachmentBytes requires params table",
                ));
            }
            let relative_path =
                get_string_field(state, params_idx, "relative_path")?.ok_or_else(|| {
                    lua_error_message(
                        "TestRemoveCachedAttachmentBytes requires params.relative_path",
                    )
                })?;
            Ok(RequestParams::TestRemoveCachedAttachmentBytes {
                params: TestRemoveCachedAttachmentBytesParams { relative_path },
            })
        }
        "TestThreadRead" | "test.thread_read" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("TestThreadRead requires params table"));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?
                .ok_or_else(|| lua_error_message("TestThreadRead requires params.account_id"))?;
            let thread_id = get_string_field(state, params_idx, "thread_id")?
                .ok_or_else(|| lua_error_message("TestThreadRead requires params.thread_id"))?;
            Ok(RequestParams::TestThreadRead {
                params: TestThreadReadParams {
                    account_id,
                    thread_id,
                },
            })
        }
        "TestPendingOpsRead" | "test.pending_ops_read" => {
            let params = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) != LuaType::Nil
            {
                if state.typ(params_idx) != LuaType::Table {
                    return Err(lua_error_message("TestPendingOpsRead params must be table"));
                }
                TestPendingOpsReadParams {
                    account_id: get_string_field(state, params_idx, "account_id")?,
                    resource_id: get_string_field(state, params_idx, "resource_id")?,
                    operation_type: get_string_field(state, params_idx, "operation_type")?,
                    status: get_string_field(state, params_idx, "status")?,
                }
            } else {
                TestPendingOpsReadParams::default()
            };
            Ok(RequestParams::TestPendingOpsRead { params })
        }
        "TestStartSync" | "test.start_sync" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("TestStartSync requires params table"));
            }
            let account_id = get_string_field(state, params_idx, "account_id")?
                .ok_or_else(|| lua_error_message("TestStartSync requires params.account_id"))?;
            Ok(RequestParams::TestStartSync {
                params: TestStartSyncParams { account_id },
            })
        }
        "TestQueryDbState" | "test.query_db_state" => {
            let params = if state.get_top() >= params_idx as usize
                && state.typ(params_idx) != LuaType::Nil
            {
                if state.typ(params_idx) != LuaType::Table {
                    return Err(lua_error_message("TestQueryDbState params must be table"));
                }
                TestQueryDbStateParams {
                    account_id: get_string_field(state, params_idx, "account_id")?,
                    message_limit: get_number_field(state, params_idx, "message_limit")?
                        .map(|value| value as u64),
                    attachment_limit: get_number_field(state, params_idx, "attachment_limit")?
                        .map(|value| value as u64),
                    calendar_limit: get_number_field(state, params_idx, "calendar_limit")?
                        .map(|value| value as u64),
                    contact_limit: get_number_field(state, params_idx, "contact_limit")?
                        .map(|value| value as u64),
                    contact_group_limit: get_number_field(
                        state,
                        params_idx,
                        "contact_group_limit",
                    )?
                    .map(|value| value as u64),
                }
            } else {
                TestQueryDbStateParams::default()
            };
            Ok(RequestParams::TestQueryDbState { params })
        }
        "TestSearchIndex" | "test.search_index" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("TestSearchIndex requires params table"));
            }
            let query = get_string_field(state, params_idx, "query")?
                .ok_or_else(|| lua_error_message("TestSearchIndex requires params.query"))?;
            Ok(RequestParams::TestSearchIndex {
                params: TestSearchIndexParams {
                    query,
                    account_id: get_string_field(state, params_idx, "account_id")?,
                    limit: get_number_field(state, params_idx, "limit")?.map(|value| value as u64),
                },
            })
        }
        "TestDelayNextWrite" | "test.delay_next_write" => {
            if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
                return Err(lua_error_message("TestDelayNextWrite requires params table"));
            }
            let kind = get_string_field(state, params_idx, "kind")?
                .ok_or_else(|| lua_error_message("TestDelayNextWrite requires params.kind"))?;
            let millis = get_number_field(state, params_idx, "millis")?
                .ok_or_else(|| lua_error_message("TestDelayNextWrite requires params.millis"))?;
            Ok(RequestParams::TestDelayNextWrite {
                params: TestDelayNextWriteParams {
                    kind,
                    millis: millis as u64,
                },
            })
        }
        other => Err(lua_error_message(format!(
            "request method {other:?} is not registered in harness"
        ))),
    }
}

fn parse_contact_save_params(
    state: &mut State,
    params_idx: isize,
) -> dellingr::Result<ContactSaveParams> {
    if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
        return Err(lua_error_message(
            "ContactsContactSave requires params table",
        ));
    }
    let id = get_string_field(state, params_idx, "id")?
        .ok_or_else(|| lua_error_message("ContactsContactSave requires params.id"))?;
    let email = get_string_field(state, params_idx, "email")?
        .ok_or_else(|| lua_error_message("ContactsContactSave requires params.email"))?;
    Ok(ContactSaveParams {
        id,
        email,
        display_name: get_string_field(state, params_idx, "display_name")?,
        email2: get_string_field(state, params_idx, "email2")?,
        phone: get_string_field(state, params_idx, "phone")?,
        company: get_string_field(state, params_idx, "company")?,
        notes: get_string_field(state, params_idx, "notes")?,
        account_id: get_string_field(state, params_idx, "account_id")?,
        account_color: get_string_field(state, params_idx, "account_color")?,
        groups: get_string_array_field(state, params_idx, "groups")?,
        source: get_string_field(state, params_idx, "source")?,
        server_id: get_string_field(state, params_idx, "server_id")?,
    })
}

fn parse_settings_set_params(
    state: &mut State,
    params_idx: isize,
) -> dellingr::Result<SettingsSetParams> {
    if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
        return Err(lua_error_message("SettingsSet requires params table"));
    }
    let top = state.get_top();
    state.push_string("values");
    state.get_table(params_idx)?;
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message("SettingsSet requires values table"));
    }
    let values_idx = state.get_top() as isize;
    let len = state.table_len(values_idx);
    let mut values = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(values_idx)?;
        if state.typ(-1) != LuaType::Table {
            state.set_top(top as isize);
            return Err(lua_error_message("setting value must be table"));
        }
        let value_idx = state.get_top() as isize;
        values.push(parse_setting_value(state, value_idx)?);
        state.pop(1);
    }
    state.set_top(top as isize);
    Ok(SettingsSetParams { values })
}

fn parse_setting_value(state: &mut State, value_idx: isize) -> dellingr::Result<SettingValue> {
    let kind = get_string_field(state, value_idx, "type")?
        .ok_or_else(|| lua_error_message("setting value requires type"))?;
    match normalize_name(&kind).as_str() {
        "showsyncstatus" => Ok(SettingValue::ShowSyncStatus(required_setting_bool(
            state, value_idx, &kind,
        )?)),
        "blockremoteimages" => Ok(SettingValue::BlockRemoteImages(required_setting_bool(
            state, value_idx, &kind,
        )?)),
        "phishingdetectionenabled" => Ok(SettingValue::PhishingDetectionEnabled(
            required_setting_bool(state, value_idx, &kind)?,
        )),
        "phishingsensitivity" => Ok(SettingValue::PhishingSensitivity(
            required_setting_string(state, value_idx, &kind)?,
        )),
        "theme" => Ok(SettingValue::Theme(required_setting_string(
            state, value_idx, &kind,
        )?)),
        "fontsize" => Ok(SettingValue::FontSize(required_setting_string(
            state, value_idx, &kind,
        )?)),
        "readingpaneposition" => Ok(SettingValue::ReadingPanePosition(
            required_setting_string(state, value_idx, &kind)?,
        )),
        "syncperioddays" => Ok(SettingValue::SyncPeriodDays(
            required_setting_string(state, value_idx, &kind)?,
        )),
        "compressattachments" => Ok(SettingValue::CompressAttachments(
            required_setting_bool(state, value_idx, &kind)?,
        )),
        "allowlossycompression" => Ok(SettingValue::AllowLossyCompression(
            required_setting_bool(state, value_idx, &kind)?,
        )),
        "openedfilescleanupdays" => Ok(SettingValue::OpenedFilesCleanupDays(
            required_setting_string(state, value_idx, &kind)?,
        )),
        other => Err(lua_error_message(format!(
            "unsupported setting value {other:?}"
        ))),
    }
}

fn required_setting_bool(
    state: &mut State,
    idx: isize,
    kind: &str,
) -> dellingr::Result<bool> {
    get_bool_field(state, idx, "value")?
        .ok_or_else(|| lua_error_message(format!("{kind} requires value")))
}

fn required_setting_string(
    state: &mut State,
    idx: isize,
    kind: &str,
) -> dellingr::Result<String> {
    get_string_field(state, idx, "value")?
        .ok_or_else(|| lua_error_message(format!("{kind} requires value")))
}

fn parse_action_plan(state: &mut State, params_idx: isize) -> dellingr::Result<ActionWirePlan> {
    if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
        return Err(lua_error_message("ActionExecutePlan requires params table"));
    }

    let top = state.get_top();
    state.push_string("plan");
    state.get_table(params_idx)?;
    let plan_idx = if state.typ(-1) == LuaType::Table {
        state.get_top() as isize
    } else {
        state.set_top(top as isize);
        params_idx
    };

    let plan_id = match get_string_field(state, plan_idx, "plan_id")? {
        Some(value) => parse_plan_id(&value)?,
        None => PlanId::new_v7(),
    };
    let operations = parse_action_operations(state, plan_idx)?;
    state.set_top(top as isize);
    Ok(ActionWirePlan {
        plan_id,
        operations,
    })
}

fn parse_calendar_action_plan(
    state: &mut State,
    params_idx: isize,
) -> dellingr::Result<CalendarActionPlan> {
    if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
        return Err(lua_error_message("CalActionExecutePlan requires params table"));
    }

    let top = state.get_top();
    state.push_string("plan");
    state.get_table(params_idx)?;
    let plan_idx = if state.typ(-1) == LuaType::Table {
        state.get_top() as isize
    } else {
        state.set_top(top as isize);
        params_idx
    };

    let plan_id = match get_string_field(state, plan_idx, "plan_id")? {
        Some(value) => parse_plan_id(&value)?,
        None => PlanId::new_v7(),
    };
    let operations = parse_calendar_action_operations(state, plan_idx)?;
    state.set_top(top as isize);
    Ok(CalendarActionPlan {
        plan_id,
        operations,
    })
}

fn parse_plan_id_request(
    state: &mut State,
    params_idx: isize,
    method: &str,
) -> dellingr::Result<PlanId> {
    if state.get_top() < params_idx as usize {
        return Err(lua_error_message(format!("{method} requires plan_id")));
    }
    if state.typ(params_idx) == LuaType::Table {
        let plan_id = get_string_field(state, params_idx, "plan_id")?
            .ok_or_else(|| lua_error_message(format!("{method} requires params.plan_id")))?;
        return parse_plan_id(&plan_id);
    }
    let plan_id = state.to_string(params_idx)?;
    parse_plan_id(&plan_id)
}

fn parse_plan_id(value: &str) -> dellingr::Result<PlanId> {
    uuid::Uuid::parse_str(value)
        .map(PlanId)
        .map_err(|error| lua_error_message(format!("invalid plan_id {value:?}: {error}")))
}

fn parse_send_request(
    state: &mut State,
    params_idx: isize,
) -> dellingr::Result<SendWireRequest> {
    if state.get_top() < params_idx as usize || state.typ(params_idx) != LuaType::Table {
        return Err(lua_error_message("ActionSend requires params table"));
    }

    let top = state.get_top();
    let send_id = get_string_field(state, params_idx, "send_id")?
        .ok_or_else(|| lua_error_message("ActionSend requires send_id"))
        .and_then(|value| parse_plan_id(&value))?;
    let from_account_id = get_string_field(state, params_idx, "from_account_id")?
        .ok_or_else(|| lua_error_message("ActionSend requires from_account_id"))?;
    let message = parse_send_message(state, params_idx)?;
    let attachments = parse_send_attachments(state, params_idx)?;
    state.set_top(top as isize);
    Ok(SendWireRequest {
        send_id,
        from_account_id,
        message,
        attachments,
    })
}

fn parse_send_message(
    state: &mut State,
    request_idx: isize,
) -> dellingr::Result<SendWireMessage> {
    let top = state.get_top();
    state.push_string("message");
    state.get_table(request_idx)?;
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message("ActionSend requires message table"));
    }
    let message_idx = state.get_top() as isize;
    let message = SendWireMessage {
        draft_id: get_string_field(state, message_idx, "draft_id")?
            .ok_or_else(|| lua_error_message("ActionSend message requires draft_id"))?,
        from: get_string_field(state, message_idx, "from")?
            .ok_or_else(|| lua_error_message("ActionSend message requires from"))?,
        to: get_string_array_field(state, message_idx, "to")?,
        cc: get_string_array_field(state, message_idx, "cc")?,
        bcc: get_string_array_field(state, message_idx, "bcc")?,
        subject: get_string_field(state, message_idx, "subject")?,
        body_html: get_string_field(state, message_idx, "body_html")?
            .ok_or_else(|| lua_error_message("ActionSend message requires body_html"))?,
        body_text: get_string_field(state, message_idx, "body_text")?
            .ok_or_else(|| lua_error_message("ActionSend message requires body_text"))?,
        in_reply_to: get_string_field(state, message_idx, "in_reply_to")?,
        references: get_string_field(state, message_idx, "references")?,
        thread_id: get_string_field(state, message_idx, "thread_id")?,
    };
    state.set_top(top as isize);
    Ok(message)
}

fn parse_send_attachments(
    state: &mut State,
    request_idx: isize,
) -> dellingr::Result<Vec<SendWireAttachment>> {
    let top = state.get_top();
    state.push_string("attachments");
    state.get_table(request_idx)?;
    if state.typ(-1) == LuaType::Nil {
        state.set_top(top as isize);
        return Ok(Vec::new());
    }
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message("ActionSend attachments must be a table"));
    }
    let attachments_idx = state.get_top() as isize;
    let len = state.table_len(attachments_idx);
    let mut attachments = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(attachments_idx)?;
        if state.typ(-1) != LuaType::Table {
            state.set_top(top as isize);
            return Err(lua_error_message("ActionSend attachment must be table"));
        }
        let att_idx = state.get_top() as isize;
        attachments.push(parse_send_attachment(state, att_idx)?);
        state.pop(1);
    }
    state.set_top(top as isize);
    Ok(attachments)
}

fn parse_send_attachment(
    state: &mut State,
    att_idx: isize,
) -> dellingr::Result<SendWireAttachment> {
    let size = get_number_field(state, att_idx, "size")?
        .ok_or_else(|| lua_error_message("ActionSend attachment requires size"))?;
    Ok(SendWireAttachment {
        source: parse_send_attachment_source(state, att_idx)?,
        size: size as u64,
        mime: get_string_field(state, att_idx, "mime")?
            .or_else(|| get_string_field(state, att_idx, "mime_type").ok().flatten())
            .ok_or_else(|| lua_error_message("ActionSend attachment requires mime"))?,
        filename: get_string_field(state, att_idx, "filename")?
            .ok_or_else(|| lua_error_message("ActionSend attachment requires filename"))?,
        content_id: get_string_field(state, att_idx, "content_id")?,
    })
}

fn parse_send_attachment_source(
    state: &mut State,
    att_idx: isize,
) -> dellingr::Result<SendAttachmentSource> {
    let top = state.get_top();
    state.push_string("source");
    state.get_table(att_idx)?;
    let source_idx = if state.typ(-1) == LuaType::Table {
        state.get_top() as isize
    } else {
        state.set_top(top as isize);
        att_idx
    };
    let kind = get_string_field(state, source_idx, "kind")?
        .unwrap_or_else(|| "staging_file".to_string());
    if kind != "staging_file" {
        state.set_top(top as isize);
        return Err(lua_error_message(format!(
            "unsupported ActionSend attachment source {kind:?}"
        )));
    }
    let relative_path = get_string_field(state, source_idx, "relative_path")?
        .ok_or_else(|| lua_error_message("ActionSend source requires relative_path"))?;
    let content_hash = get_content_hash_field(state, source_idx, "content_hash")?
        .or_else(|| get_content_hash_field(state, source_idx, "content_hash_hex").ok().flatten())
        .ok_or_else(|| lua_error_message("ActionSend source requires content_hash"))?;
    state.set_top(top as isize);
    Ok(SendAttachmentSource::StagingFile {
        relative_path,
        content_hash,
    })
}

fn get_content_hash_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Option<[u8; 32]>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = match state.typ(-1) {
        LuaType::Nil => None,
        LuaType::String => Some(parse_hex_hash(&state.to_string(-1)?)?),
        LuaType::Table => {
            let values_idx = state.get_top() as isize;
            let len = state.table_len(values_idx);
            if len != 32 {
                state.set_top(top as isize);
                return Err(lua_error_message(format!(
                    "{key} must have 32 byte values, got {len}"
                )));
            }
            let mut bytes = [0_u8; 32];
            for i in 1..=len {
                state.push_number(i as f64);
                state.get_table(values_idx)?;
                let value = state.to_number(-1)?;
                if !(0.0..=255.0).contains(&value) {
                    state.set_top(top as isize);
                    return Err(lua_error_message(format!(
                        "{key}[{i}] byte out of range: {value}"
                    )));
                }
                bytes[i - 1] = value as u8;
                state.pop(1);
            }
            Some(bytes)
        }
        other => {
            state.set_top(top as isize);
            return Err(lua_error_message(format!(
                "{key} must be a 32-byte table or 64-char hex string, got {}",
                other.as_str()
            )));
        }
    };
    state.set_top(top as isize);
    Ok(result)
}

fn parse_hex_hash(value: &str) -> dellingr::Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(lua_error_message(format!(
            "content_hash hex must be 64 chars, got {}",
            value.len()
        )));
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&value[start..start + 2], 16)
            .map_err(|error| lua_error_message(format!("invalid content_hash hex: {error}")))?;
    }
    Ok(out)
}

fn parse_action_operations(
    state: &mut State,
    plan_idx: isize,
) -> dellingr::Result<Vec<ActionWireOperation>> {
    let top = state.get_top();
    state.push_string("operations");
    state.get_table(plan_idx)?;
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message("ActionExecutePlan requires operations table"));
    }
    let operations_idx = state.get_top() as isize;
    let len = state.table_len(operations_idx);
    let mut operations = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(operations_idx)?;
        if state.typ(-1) != LuaType::Table {
            state.set_top(top as isize);
            return Err(lua_error_message("action operation must be table"));
        }
        let op_idx = state.get_top() as isize;
        let operation = parse_action_operation(state, op_idx, i)?;
        operations.push(operation);
        state.pop(1);
    }
    state.set_top(top as isize);
    Ok(operations)
}

fn parse_calendar_action_operations(
    state: &mut State,
    plan_idx: isize,
) -> dellingr::Result<Vec<CalendarActionWireOperation>> {
    let top = state.get_top();
    state.push_string("operations");
    state.get_table(plan_idx)?;
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message(
            "CalActionExecutePlan requires operations table",
        ));
    }
    let operations_idx = state.get_top() as isize;
    let len = state.table_len(operations_idx);
    let mut operations = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(operations_idx)?;
        if state.typ(-1) != LuaType::Table {
            state.set_top(top as isize);
            return Err(lua_error_message("calendar action operation must be table"));
        }
        let op_idx = state.get_top() as isize;
        let operation = parse_calendar_action_operation(state, op_idx, i)?;
        operations.push(operation);
        state.pop(1);
    }
    state.set_top(top as isize);
    Ok(operations)
}

fn parse_action_operation(
    state: &mut State,
    op_idx: isize,
    ordinal: usize,
) -> dellingr::Result<ActionWireOperation> {
    let operation_id = get_number_field(state, op_idx, "operation_id")?
        .map(|value| value as u32)
        .unwrap_or_else(|| (ordinal - 1) as u32);
    let account_id = get_string_field(state, op_idx, "account_id")?
        .ok_or_else(|| lua_error_message("action operation requires account_id"))?;
    let thread_id = get_string_field(state, op_idx, "thread_id")?
        .ok_or_else(|| lua_error_message("action operation requires thread_id"))?;
    let op_name = get_string_field(state, op_idx, "operation")?
        .or_else(|| get_string_field(state, op_idx, "kind").ok().flatten())
        .ok_or_else(|| lua_error_message("action operation requires operation"))?;
    let operation = parse_wire_mail_operation(state, op_idx, &op_name)?;
    Ok(ActionWireOperation {
        operation_id: OperationId(operation_id),
        account_id,
        thread_id,
        operation,
    })
}

fn parse_calendar_action_operation(
    state: &mut State,
    op_idx: isize,
    ordinal: usize,
) -> dellingr::Result<CalendarActionWireOperation> {
    let operation_id = get_number_field(state, op_idx, "operation_id")?
        .map(|value| value as u32)
        .unwrap_or_else(|| (ordinal - 1) as u32);
    let account_id = get_string_field(state, op_idx, "account_id")?
        .ok_or_else(|| lua_error_message("calendar action operation requires account_id"))?;
    let op_name = get_string_field(state, op_idx, "operation")?
        .or_else(|| get_string_field(state, op_idx, "kind").ok().flatten())
        .ok_or_else(|| lua_error_message("calendar action operation requires operation"))?;
    let operation = parse_wire_calendar_operation(state, op_idx, &op_name)?;
    Ok(CalendarActionWireOperation {
        operation_id: OperationId(operation_id),
        account_id,
        operation,
    })
}

fn parse_wire_calendar_operation(
    state: &mut State,
    op_idx: isize,
    op_name: &str,
) -> dellingr::Result<WireCalendarOperation> {
    match normalize_name(op_name).as_str() {
        "createevent" | "create" => {
            let calendar_id = get_string_field(state, op_idx, "calendar_id")?
                .or_else(|| get_string_field(state, op_idx, "calendar_remote_id").ok().flatten())
                .ok_or_else(|| {
                    lua_error_message("CreateEvent requires calendar_id")
                })?;
            Ok(WireCalendarOperation::CreateEvent {
                calendar_remote_id: calendar_id,
                input: parse_wire_calendar_event_input(state, op_idx, op_name)?,
            })
        }
        "updateevent" | "update" => {
            let event_id = get_string_field(state, op_idx, "event_id")?
                .ok_or_else(|| lua_error_message("UpdateEvent requires event_id"))?;
            Ok(WireCalendarOperation::UpdateEvent {
                event_id,
                input: parse_wire_calendar_event_input(state, op_idx, op_name)?,
            })
        }
        "deleteevent" | "delete" => {
            let event_id = get_string_field(state, op_idx, "event_id")?
                .ok_or_else(|| lua_error_message("DeleteEvent requires event_id"))?;
            Ok(WireCalendarOperation::DeleteEvent { event_id })
        }
        other => Err(lua_error_message(format!(
            "unknown calendar operation {other:?}"
        ))),
    }
}

fn parse_wire_calendar_event_input(
    state: &mut State,
    op_idx: isize,
    op_name: &str,
) -> dellingr::Result<WireCalendarEventInput> {
    let top = state.get_top();
    state.push_string("input");
    state.get_table(op_idx)?;
    let input_idx = if state.typ(-1) == LuaType::Table {
        state.get_top() as isize
    } else {
        state.set_top(top as isize);
        op_idx
    };

    let title = get_first_string_field(state, input_idx, &["title", "summary"])?
        .ok_or_else(|| lua_error_message(format!("{op_name} requires input.title")))?;
    let start_time = get_first_number_field(state, input_idx, &["start_time", "start"])?
        .ok_or_else(|| lua_error_message(format!("{op_name} requires input.start_time")))?
        as i64;
    let end_time = get_first_number_field(state, input_idx, &["end_time", "end"])?
        .ok_or_else(|| lua_error_message(format!("{op_name} requires input.end_time")))?
        as i64;
    let input = WireCalendarEventInput {
        title,
        description: get_string_field(state, input_idx, "description")?.unwrap_or_default(),
        location: get_string_field(state, input_idx, "location")?.unwrap_or_default(),
        start_time,
        end_time,
        is_all_day: get_bool_field(state, input_idx, "is_all_day")?
            .or_else(|| get_bool_field(state, input_idx, "isAllDay").ok().flatten())
            .unwrap_or(false),
        timezone: get_string_field(state, input_idx, "timezone")?,
        recurrence_rule: get_string_field(state, input_idx, "recurrence_rule")?
            .or_else(|| get_string_field(state, input_idx, "recurrenceRule").ok().flatten()),
        availability: get_string_field(state, input_idx, "availability")?,
        visibility: get_string_field(state, input_idx, "visibility")?,
    };
    state.set_top(top as isize);
    Ok(input)
}

fn parse_wire_mail_operation(
    state: &mut State,
    op_idx: isize,
    op_name: &str,
) -> dellingr::Result<WireMailOperation> {
    match normalize_name(op_name).as_str() {
        "archive" => Ok(WireMailOperation::Archive),
        "trash" => Ok(WireMailOperation::Trash),
        "permanentdelete" => Ok(WireMailOperation::PermanentDelete),
        "setspam" => Ok(WireMailOperation::SetSpam {
            to: required_bool_field(state, op_idx, "to", op_name)?,
        }),
        "setstarred" => Ok(WireMailOperation::SetStarred {
            to: required_bool_field(state, op_idx, "to", op_name)?,
        }),
        "setread" => Ok(WireMailOperation::SetRead {
            to: required_bool_field(state, op_idx, "to", op_name)?,
        }),
        "setpinned" => Ok(WireMailOperation::SetPinned {
            to: required_bool_field(state, op_idx, "to", op_name)?,
        }),
        "setmuted" => Ok(WireMailOperation::SetMuted {
            to: required_bool_field(state, op_idx, "to", op_name)?,
        }),
        "movetofolder" => {
            let dest = get_string_field(state, op_idx, "dest")?
                .ok_or_else(|| lua_error_message("MoveToFolder requires dest"))?;
            let source = get_string_field(state, op_idx, "source")?.map(WireFolderId);
            Ok(WireMailOperation::MoveToFolder {
                dest: WireFolderId(dest),
                source,
            })
        }
        "addlabel" => {
            let label_id = get_string_field(state, op_idx, "label_id")?
                .ok_or_else(|| lua_error_message("AddLabel requires label_id"))?;
            Ok(WireMailOperation::AddLabel {
                label_id: WireTagId(label_id),
            })
        }
        "removelabel" => {
            let label_id = get_string_field(state, op_idx, "label_id")?
                .ok_or_else(|| lua_error_message("RemoveLabel requires label_id"))?;
            Ok(WireMailOperation::RemoveLabel {
                label_id: WireTagId(label_id),
            })
        }
        "snooze" => {
            let until = get_number_field(state, op_idx, "until")?
                .ok_or_else(|| lua_error_message("Snooze requires until"))?;
            Ok(WireMailOperation::Snooze {
                until: until as i64,
            })
        }
        "unsnooze" => Ok(WireMailOperation::Unsnooze),
        other => Err(lua_error_message(format!(
            "unsupported action operation {other:?}"
        ))),
    }
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-' && *ch != '.')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn required_bool_field(
    state: &mut State,
    idx: isize,
    key: &str,
    op_name: &str,
) -> dellingr::Result<bool> {
    get_bool_field(state, idx, key)?
        .ok_or_else(|| lua_error_message(format!("{op_name} requires {key}")))
}

fn read_extra_args(state: &mut State, idx: isize) -> dellingr::Result<Vec<String>> {
    if state.get_top() < idx as usize || state.typ(idx) == LuaType::Nil {
        return Ok(Vec::new());
    }
    if state.typ(idx) != LuaType::Table {
        return Err(lua_error_message("extra args must be a table"));
    }
    let len = state.table_len(idx);
    let mut args = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(idx)?;
        args.push(state.to_string(-1)?);
        state.pop(1);
    }
    Ok(args)
}

fn push_json(state: &mut State, value: &serde_json::Value) -> dellingr::Result<()> {
    match value {
        serde_json::Value::Null => state.push_nil(),
        serde_json::Value::Bool(value) => state.push_boolean(*value),
        serde_json::Value::Number(value) => {
            state.push_number(value.as_f64().unwrap_or(0.0));
        }
        serde_json::Value::String(value) => state.push_string(value),
        serde_json::Value::Array(values) => {
            state.new_table();
            let idx = state.get_top() as isize;
            for (offset, item) in values.iter().enumerate() {
                state.push_number((offset + 1) as f64);
                push_json(state, item)?;
                state.set_table_raw(idx)?;
            }
        }
        serde_json::Value::Object(values) => {
            state.new_table();
            let idx = state.get_top() as isize;
            for (key, item) in values {
                state.push_string(key);
                push_json(state, item)?;
                state.set_table_raw(idx)?;
            }
        }
    }
    Ok(())
}

enum LuaJsonKey {
    ArrayIndex(usize),
    ObjectKey(String),
}

fn lua_value_to_json(state: &mut State, idx: isize) -> dellingr::Result<serde_json::Value> {
    let idx = absolute_stack_idx(state, idx);
    match state.typ(idx) {
        LuaType::Nil => Ok(serde_json::Value::Null),
        LuaType::Boolean => Ok(serde_json::Value::Bool(state.to_boolean(idx))),
        LuaType::Number => {
            let number = state.to_number(idx)?;
            lua_number_to_json(number)
        }
        LuaType::String => state.to_string(idx).map(serde_json::Value::String),
        LuaType::Table => lua_table_to_json(state, idx),
        LuaType::Function => Err(lua_error_message("function is not JSON-serializable")),
    }
}

fn lua_number_to_json(number: f64) -> dellingr::Result<serde_json::Value> {
    const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_991.0;
    if !number.is_finite() {
        return Err(lua_error_message(format!(
            "number is not JSON-safe: {number}"
        )));
    }
    if number.fract() == 0.0 && number.abs() <= MAX_EXACT_INTEGER {
        return Ok(serde_json::Value::Number(serde_json::Number::from(
            number as i64,
        )));
    }
    serde_json::Number::from_f64(number)
        .map(serde_json::Value::Number)
        .ok_or_else(|| lua_error_message(format!("number is not JSON-safe: {number}")))
}

fn lua_table_to_json(state: &mut State, idx: isize) -> dellingr::Result<serde_json::Value> {
    let idx = absolute_stack_idx(state, idx);
    let top = state.get_top();
    let mut array_entries = Vec::new();
    let mut object_entries = Vec::new();
    state.push_nil();
    loop {
        let has_next = match state.table_next(idx) {
            Ok(has_next) => has_next,
            Err(error) => {
                state.set_top(top as isize);
                return Err(error);
            }
        };
        if !has_next {
            break;
        }
        let key = match lua_json_key(state, -2) {
            Ok(key) => key,
            Err(error) => {
                state.set_top(top as isize);
                return Err(error);
            }
        };
        let value = match lua_value_to_json(state, -1) {
            Ok(value) => value,
            Err(error) => {
                state.set_top(top as isize);
                return Err(error);
            }
        };
        match key {
            LuaJsonKey::ArrayIndex(index) => array_entries.push((index, value)),
            LuaJsonKey::ObjectKey(key) => object_entries.push((key, value)),
        }
        state.pop(1);
    }
    state.set_top(top as isize);

    if object_entries.is_empty() && !array_entries.is_empty() {
        array_entries.sort_by_key(|(index, _)| *index);
        let mut values = Vec::with_capacity(array_entries.len());
        for (offset, (index, value)) in array_entries.into_iter().enumerate() {
            let expected = offset + 1;
            if index != expected {
                return Err(lua_error_message(format!(
                    "JSON array table must have contiguous 1-based indices; \
                     expected {expected}, got {index}"
                )));
            }
            values.push(value);
        }
        Ok(serde_json::Value::Array(values))
    } else if array_entries.is_empty() {
        let mut object = serde_json::Map::new();
        for (key, value) in object_entries {
            object.insert(key, value);
        }
        Ok(serde_json::Value::Object(object))
    } else {
        Err(lua_error_message(
            "JSON table cannot mix array indices and object keys",
        ))
    }
}

fn lua_json_key(state: &mut State, idx: isize) -> dellingr::Result<LuaJsonKey> {
    let idx = absolute_stack_idx(state, idx);
    match state.typ(idx) {
        LuaType::Number => {
            let number = state.to_number(idx)?;
            if !number.is_finite() || number < 1.0 || number.fract() != 0.0 {
                return Err(lua_error_message(format!(
                    "JSON array index must be a positive integer, got {number}"
                )));
            }
            Ok(LuaJsonKey::ArrayIndex(number as usize))
        }
        LuaType::String => state.to_string(idx).map(LuaJsonKey::ObjectKey),
        other => Err(lua_error_message(format!(
            "JSON object key must be string or array index, got {}",
            other.as_str()
        ))),
    }
}

fn absolute_stack_idx(state: &State, idx: isize) -> isize {
    if idx < 0 {
        state.get_top() as isize + idx + 1
    } else {
        idx
    }
}

fn set_field_fn(
    state: &mut State,
    table_idx: isize,
    key: &str,
    func: dellingr::RustFunc,
) -> dellingr::Result<()> {
    state.push_string(key);
    state.push_rust_fn(func);
    state.set_table_raw(table_idx)
}

fn set_field_string(
    state: &mut State,
    table_idx: isize,
    key: &str,
    value: &str,
) -> dellingr::Result<()> {
    state.push_string(key);
    state.push_string(value);
    state.set_table_raw(table_idx)
}

fn set_field_number(
    state: &mut State,
    table_idx: isize,
    key: &str,
    value: f64,
) -> dellingr::Result<()> {
    state.push_string(key);
    state.push_number(value);
    state.set_table_raw(table_idx)
}

fn set_pushed_field(state: &mut State, table_idx: isize, key: &str) -> dellingr::Result<()> {
    state.push_string(key);
    state.insert(-2)?;
    state.set_table_raw(table_idx)
}

fn get_string_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Option<String>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = if state.typ(-1) == LuaType::Nil {
        None
    } else {
        Some(state.to_string(-1)?)
    };
    state.set_top(top as isize);
    Ok(result)
}

fn get_first_string_field(
    state: &mut State,
    table_idx: isize,
    keys: &[&str],
) -> dellingr::Result<Option<String>> {
    for key in keys {
        if let Some(value) = get_string_field(state, table_idx, key)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn get_number_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Option<f64>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = if state.typ(-1) == LuaType::Nil {
        None
    } else {
        Some(state.to_number(-1)?)
    };
    state.set_top(top as isize);
    Ok(result)
}

fn get_first_number_field(
    state: &mut State,
    table_idx: isize,
    keys: &[&str],
) -> dellingr::Result<Option<f64>> {
    for key in keys {
        if let Some(value) = get_number_field(state, table_idx, key)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn get_bool_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Option<bool>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    let result = if state.typ(-1) == LuaType::Nil {
        None
    } else {
        Some(state.to_boolean(-1))
    };
    state.set_top(top as isize);
    Ok(result)
}

fn get_string_array_field(
    state: &mut State,
    table_idx: isize,
    key: &str,
) -> dellingr::Result<Vec<String>> {
    let top = state.get_top();
    state.push_string(key);
    state.get_table(table_idx)?;
    if state.typ(-1) == LuaType::Nil {
        state.set_top(top as isize);
        return Ok(Vec::new());
    }
    if state.typ(-1) != LuaType::Table {
        state.set_top(top as isize);
        return Err(lua_error_message(format!("{key} must be a table")));
    }
    let values_idx = state.get_top() as isize;
    let len = state.table_len(values_idx);
    let mut values = Vec::with_capacity(len);
    for i in 1..=len {
        state.push_number(i as f64);
        state.get_table(values_idx)?;
        values.push(state.to_string(-1)?);
        state.pop(1);
    }
    state.set_top(top as isize);
    Ok(values)
}

fn resource_id(state: &mut State, idx: isize) -> dellingr::Result<u64> {
    if state.typ(idx) != LuaType::Table {
        return Err(lua_error_message("expected harness resource table"));
    }
    let id = get_number_field(state, idx, "__harness_id")?
        .ok_or_else(|| lua_error_message("resource missing __harness_id"))?;
    Ok(id as u64)
}

fn context(state: &mut State) -> dellingr::Result<Arc<Mutex<HarnessContext>>> {
    state
        .user_data::<Arc<Mutex<HarnessContext>>>()
        .cloned()
        .ok_or_else(|| lua_error_message("harness context missing"))
}

fn signal_number(state: &mut State) -> dellingr::Result<i32> {
    if state.typ(2) == LuaType::Number {
        return Ok(state.to_number(2)? as i32);
    }
    match state.to_string(2)?.as_str() {
        "SIGKILL" | "KILL" => Ok(libc::SIGKILL),
        "SIGTERM" | "TERM" => Ok(libc::SIGTERM),
        other => Err(lua_error_message(format!("unknown signal {other:?}"))),
    }
}

fn duration_from_seconds(seconds: f64) -> Duration {
    Duration::from_millis((seconds * 1000.0).max(0.0) as u64)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn boot_code_name(code: BootExitCode) -> &'static str {
    match code {
        BootExitCode::HandshakeFailure => "HandshakeFailure",
        BootExitCode::AnotherInstanceRunning => "AnotherInstanceRunning",
        BootExitCode::MigrationFailure => "MigrationFailure",
        BootExitCode::KeyLoadFailure => "KeyLoadFailure",
        BootExitCode::LockIoFailure => "LockIoFailure",
    }
}

fn boot_phase_kind_name(kind: BootPhaseKind) -> &'static str {
    match kind {
        BootPhaseKind::LoadingKey => "LoadingKey",
        BootPhaseKind::OpeningDatabase => "OpeningDatabase",
        BootPhaseKind::Migrating => "Migrating",
        BootPhaseKind::RecoveringPendingOps => "RecoveringPendingOps",
        BootPhaseKind::SweepingQueuedDrafts => "SweepingQueuedDrafts",
        BootPhaseKind::BackfillingThreadParticipants => "BackfillingThreadParticipants",
        BootPhaseKind::DrainingDraftWal => "DrainingDraftWal",
        BootPhaseKind::OpeningBodyAndInlineStores => "OpeningBodyAndInlineStores",
        BootPhaseKind::OpeningSearchIndex => "OpeningSearchIndex",
        BootPhaseKind::RunningInvariantPass => "RunningInvariantPass",
    }
}

fn table_summary_json(state: &mut State, idx: isize) -> dellingr::Result<serde_json::Value> {
    let typ = get_string_field(state, idx, "type")?.unwrap_or_else(|| "unknown".to_string());
    Ok(serde_json::json!({ "type": typ }))
}

fn parse_ceiling(source: &str) -> Option<Duration> {
    for line in source.lines().take(16) {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("--") else {
            continue;
        };
        let rest = rest.trim();
        let Some(value) = rest.strip_prefix("ceiling:") else {
            continue;
        };
        return parse_duration(value.trim());
    }
    None
}

fn parse_duration(value: &str) -> Option<Duration> {
    if let Some(secs) = value.strip_suffix('s') {
        return secs.trim().parse::<u64>().ok().map(Duration::from_secs);
    }
    if let Some(ms) = value.strip_suffix("ms") {
        return ms.trim().parse::<u64>().ok().map(Duration::from_millis);
    }
    value.parse::<u64>().ok().map(Duration::from_secs)
}

fn artefact_dir() -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BROKKR_HARNESS_ARTEFACT_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(std::env::current_dir()?
        .join("target")
        .join(format!("service-harness-{}", std::process::id())))
}

fn app_binary_path() -> std::io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("BROKKR_TEST_BIN_DIR") {
        let candidate = PathBuf::from(dir).join("app");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    std::env::current_exe()
}

fn finish_context(
    context: &Arc<Mutex<HarnessContext>>,
    success: bool,
    error: Option<String>,
) {
    context
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .finish(success, error);
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ty.is_file() {
            let _ = std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> std::io::Result<bool> {
    let signal_pid = i32::try_from(pid).map_err(std::io::Error::other)?;
    let result = unsafe { libc::kill(signal_pid, 0) };
    if result == 0 {
        #[cfg(target_os = "linux")]
        if let Some(alive) = linux_pid_has_live_state(pid)? {
            return Ok(alive);
        }
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(err),
    }
}

#[cfg(target_os = "linux")]
fn linux_pid_has_live_state(pid: u32) -> std::io::Result<Option<bool>> {
    let path = format!("/proc/{pid}/stat");
    let stat = match std::fs::read_to_string(path) {
        Ok(stat) => stat,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Some(false)),
        Err(err) => return Err(err),
    };
    let Some(end_comm) = stat.rfind(") ") else {
        return Ok(None);
    };
    Ok(stat[end_comm + 2..].chars().next().map(|state| state != 'Z'))
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> std::io::Result<bool> {
    Ok(false)
}

fn lua_io(error: std::io::Error) -> dellingr::error::Error {
    lua_error_message(error.to_string())
}

fn lua_json(error: serde_json::Error) -> dellingr::error::Error {
    lua_error_message(error.to_string())
}

fn box_lua_error(error: dellingr::error::Error) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::other(error.to_string()).into()
}

fn lua_error_message(message: impl Into<String>) -> dellingr::error::Error {
    dellingr::error::Error::without_location(ErrorKind::InternalError(message.into()))
}
