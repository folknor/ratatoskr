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
use dellingr::error::ErrorKind;
use dellingr::{ArgCount, LuaType, RetCount, State};
use service_api::{
    BootClassification, BootExitCode, BootPhaseKind, Notification, RequestParams,
    TestCrashAfterNWritesParams, TestSeedAccountParams,
};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
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
    set_field_fn(state, table_idx, "kill", lua_kill)?;
    set_field_fn(state, table_idx, "pid_is_alive", lua_pid_is_alive)?;
    set_field_fn(state, table_idx, "sleep", lua_sleep)?;
    set_field_fn(state, table_idx, "now_ms", lua_now_ms)?;
    set_field_fn(state, table_idx, "assert", lua_assert)?;
    set_field_fn(state, table_idx, "assert_eq", lua_assert_eq)?;
    set_field_fn(state, table_idx, "same_client", lua_same_client)?;
    set_field_fn(state, table_idx, "expect_quiet", lua_expect_quiet)?;
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
    set_field_fn(state, idx, "shutdown", lua_client_shutdown)?;
    set_field_fn(state, idx, "child_pid", lua_client_child_pid)?;
    set_field_fn(state, idx, "current_generation", lua_client_current_generation)?;
    set_field_fn(state, idx, "set_respawn_args", lua_client_set_respawn_args)?;
    set_field_fn(state, idx, "notifications", lua_client_notifications)?;
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
        other => {
            set_field_string(state, idx, "type", other.method_name())?;
        }
    }
    push_json(state, &serde_json::to_value(notification).map_err(lua_json)?)?;
    set_pushed_field(state, idx, "raw")?;
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
        other => Err(lua_error_message(format!(
            "request method {other:?} is not registered in harness"
        ))),
    }
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
    let pid = i32::try_from(pid).map_err(std::io::Error::other)?;
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(err),
    }
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
