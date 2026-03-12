use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::{stream, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, Signal, System};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{broadcast, Mutex},
};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

const MAX_LOG_LINES: usize = 5_000;

#[derive(Clone)]
struct AppState {
    system: Arc<Mutex<System>>,
    managed: Arc<Mutex<HashMap<Uuid, ManagedProcessState>>>,
    notes: Arc<Mutex<HashMap<String, ProcessNoteRecord>>>,
}

#[derive(Clone)]
struct ManagedProcessState {
    metadata: ManagedProcessSummary,
    tail_logs: Arc<Mutex<VecDeque<ManagedLogLine>>>,
    log_path: PathBuf,
    broadcaster: broadcast::Sender<ManagedLogLine>,
    child: Arc<Mutex<Option<Child>>>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProcessSummary {
    pid: u32,
    parent_pid: Option<u32>,
    instance_id: String,
    name: String,
    cpu_percent: f32,
    memory_bytes: u64,
    virtual_memory_bytes: u64,
    status: String,
    started_at: Option<DateTime<Utc>>,
    command_line: String,
    executable_path: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessDetail {
    summary: ProcessSummary,
    capabilities: ProcessCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessCapabilities {
    can_terminate: bool,
    has_managed_logs: bool,
    open_files_supported: bool,
    gpu_supported: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProcessNoteRecord {
    instance_id: String,
    note: String,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ManagedProcessSummary {
    run_id: Uuid,
    pid: Option<u32>,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    started_at: DateTime<Utc>,
    status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ManagedLogLine {
    offset: u64,
    timestamp: DateTime<Utc>,
    stream: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunManagedProcessRequest {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProcessNoteRequest {
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogQuery {
    offset: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    managed_runs: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminateResponse {
    success: bool,
}

#[tokio::main]
async fn main() {
    let system = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    let state = AppState {
        system: Arc::new(Mutex::new(system)),
        managed: Arc::new(Mutex::new(HashMap::new())),
        notes: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/processes", get(list_processes))
        .route("/api/processes/:pid", get(get_process))
        .route("/api/processes/:pid/note", post(update_process_note))
        .route("/api/processes/:pid/actions/terminate", post(terminate_process))
        .route("/api/managed-processes", get(list_managed_processes).post(start_managed_process))
        .route("/api/managed-processes/:run_id/logs", get(get_managed_logs))
        .route("/api/managed-processes/:run_id/logs/stream", get(stream_managed_logs))
        .route(
            "/api/managed-processes/:run_id/actions/terminate",
            post(terminate_managed_process),
        )
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7001));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind collector listener");

    println!("collector listening on http://{}", addr);
    axum::serve(listener, app)
        .await
        .expect("collector server failed");
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let managed = state.managed.lock().await;
    Json(HealthResponse {
        status: "ok",
        managed_runs: managed.len(),
    })
}

async fn list_processes(State(state): State<AppState>) -> Json<Vec<ProcessSummary>> {
    let mut system = state.system.lock().await;
    system.refresh_processes();

    let instance_ids: HashSet<String> = system
        .processes()
        .values()
        .map(process_instance_id)
        .collect();

    let mut notes = state.notes.lock().await;
    notes.retain(|instance_id, _| instance_ids.contains(instance_id));

    let mut processes: Vec<ProcessSummary> = system
        .processes()
        .values()
        .map(|process| {
            let instance_id = process_instance_id(process);
            let note = notes.get(&instance_id).map(|record| record.note.clone());
            process_to_summary(process, note)
        })
        .collect();
    processes.sort_by(|left, right| right.cpu_percent.total_cmp(&left.cpu_percent));

    Json(processes)
}

async fn get_process(
    Path(pid): Path<u32>,
    State(state): State<AppState>,
) -> Result<Json<ProcessDetail>, (StatusCode, String)> {
    let mut system = state.system.lock().await;
    system.refresh_processes();
    let pid = Pid::from_u32(pid);

    let process = system
        .process(pid)
        .ok_or((StatusCode::NOT_FOUND, "process not found".to_string()))?;

    let instance_id = process_instance_id(process);

    let managed = state.managed.lock().await;
    let has_managed_logs = managed
        .values()
        .any(|entry| entry.metadata.pid == Some(pid.as_u32()));
    drop(managed);

    let notes = state.notes.lock().await;
    let note = notes.get(&instance_id).map(|record| record.note.clone());

    let detail = ProcessDetail {
        summary: process_to_summary(process, note),
        capabilities: ProcessCapabilities {
            can_terminate: true,
            has_managed_logs,
            open_files_supported: false,
            gpu_supported: false,
        },
    };

    Ok(Json(detail))
}

async fn update_process_note(
    Path(pid): Path<u32>,
    State(state): State<AppState>,
    Json(request): Json<UpdateProcessNoteRequest>,
) -> Result<Json<Option<ProcessNoteRecord>>, (StatusCode, String)> {
    let mut system = state.system.lock().await;
    system.refresh_processes();

    let process = system
        .process(Pid::from_u32(pid))
        .ok_or((StatusCode::NOT_FOUND, "process not found".to_string()))?;
    let instance_id = process_instance_id(process);
    let trimmed = request.note.trim().to_string();
    drop(system);

    let mut notes = state.notes.lock().await;
    if trimmed.is_empty() {
        notes.remove(&instance_id);
        return Ok(Json(None));
    }

    let record = ProcessNoteRecord {
        instance_id: instance_id.clone(),
        note: trimmed,
        updated_at: Utc::now(),
    };
    notes.insert(instance_id, record.clone());

    Ok(Json(Some(record)))
}

async fn terminate_process(
    Path(pid): Path<u32>,
    State(state): State<AppState>,
) -> Result<Json<TerminateResponse>, (StatusCode, String)> {
    let mut system = state.system.lock().await;
    system.refresh_processes();
    let pid = Pid::from_u32(pid);
    let process = system
        .process(pid)
        .ok_or((StatusCode::NOT_FOUND, "process not found".to_string()))?;

    let success = process.kill_with(Signal::Term).unwrap_or(false);
    Ok(Json(TerminateResponse { success }))
}

async fn list_managed_processes(State(state): State<AppState>) -> Json<Vec<ManagedProcessSummary>> {
    let managed = state.managed.lock().await;
    let mut runs: Vec<ManagedProcessSummary> = managed
        .values()
        .map(|entry| entry.metadata.clone())
        .collect();
    runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    Json(runs)
}

async fn start_managed_process(
    State(state): State<AppState>,
    Json(request): Json<RunManagedProcessRequest>,
) -> Result<Json<ManagedProcessSummary>, (StatusCode, String)> {
    if request.command.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "command is required".to_string()));
    }

    let mut command = Command::new(&request.command);
    command.args(&request.args);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }

    if let Some(env) = &request.env {
        command.envs(env.iter());
    }

    let mut child = command
        .spawn()
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;

    let run_id = Uuid::new_v4();
    let metadata = ManagedProcessSummary {
        run_id,
        pid: child.id(),
        command: request.command,
        args: request.args,
        cwd: request.cwd,
        started_at: Utc::now(),
        status: "running".to_string(),
    };

    let data_dir = PathBuf::from("data/managed-logs");
    fs::create_dir_all(&data_dir)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let log_path = data_dir.join(format!("{}.jsonl", run_id));
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let log_writer = Arc::new(Mutex::new(log_file));

    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let next_offset = Arc::new(Mutex::new(0));
    let (broadcaster, _) = broadcast::channel(256);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child_handle = Arc::new(Mutex::new(Some(child)));

    let state_entry = ManagedProcessState {
        metadata: metadata.clone(),
        tail_logs: Arc::clone(&logs),
        log_path: log_path.clone(),
        broadcaster: broadcaster.clone(),
        child: Arc::clone(&child_handle),
    };

    state.managed.lock().await.insert(run_id, state_entry);

    if let Some(stdout) = stdout {
        tokio::spawn(capture_stream(
            "stdout".to_string(),
            stdout,
            Arc::clone(&logs),
            Arc::clone(&next_offset),
            log_path.clone(),
            Arc::clone(&log_writer),
            broadcaster.clone(),
        ));
    }

    if let Some(stderr) = stderr {
        tokio::spawn(capture_stream(
            "stderr".to_string(),
            stderr,
            Arc::clone(&logs),
            Arc::clone(&next_offset),
            log_path.clone(),
            Arc::clone(&log_writer),
            broadcaster.clone(),
        ));
    }

    let managed_map = Arc::clone(&state.managed);
    tokio::spawn(async move {
        let exit_status = {
            let mut child_lock = child_handle.lock().await;
            if let Some(child) = child_lock.as_mut() {
                child.wait().await.ok()
            } else {
                None
            }
        };

        let mut managed = managed_map.lock().await;
        if let Some(entry) = managed.get_mut(&run_id) {
            let status = exit_status
                .map(|value| format!("exited({})", value.code().unwrap_or_default()))
                .unwrap_or_else(|| "terminated".to_string());
            entry.metadata.status = status;
        }
    });

    Ok(Json(metadata))
}

async fn get_managed_logs(
    Path(run_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<ManagedLogLine>>, (StatusCode, String)> {
    let managed = state.managed.lock().await;
    let entry = managed
        .get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, "managed process not found".to_string()))?;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(200).min(1_000);
    let serialized = fs::read_to_string(&entry.log_path)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let result: Vec<ManagedLogLine> = serialized
        .trim()
        .split('\n')
        .flat_map(|line| serde_json::Deserializer::from_str(line).into_iter::<ManagedLogLine>())
        .filter_map(Result::ok)
        .filter(|line| line.offset >= offset)
        .take(limit)
        .collect();

    Ok(Json(result))
}

async fn stream_managed_logs(
    Path(run_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, String)> {
    let managed = state.managed.lock().await;
    let entry = managed
        .get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, "managed process not found".to_string()))?;

    let initial_logs = entry.tail_logs.lock().await.iter().cloned().collect::<Vec<_>>();
    let receiver = entry.broadcaster.subscribe();

    let initial_stream = stream::iter(initial_logs.into_iter().map(|line| {
        Ok(Event::default().event("log").data(
            serde_json::to_string(&line).unwrap_or_else(|_| "{}".to_string()),
        ))
    }));

    let live_stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(line) => {
                    let event = Event::default()
                        .event("log")
                        .data(serde_json::to_string(&line).unwrap_or_else(|_| "{}".to_string()));
                    return Some((Ok(event), receiver));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(Sse::new(initial_stream.chain(live_stream)).keep_alive(KeepAlive::default()))
}

async fn terminate_managed_process(
    Path(run_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<TerminateResponse>, (StatusCode, String)> {
    let managed = state.managed.lock().await;
    let entry = managed
        .get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, "managed process not found".to_string()))?;

    let mut child = entry.child.lock().await;
    let success = if let Some(process) = child.as_mut() {
        process.kill().await.is_ok()
    } else {
        false
    };

    Ok(Json(TerminateResponse { success }))
}

async fn capture_stream<T>(
    stream_name: String,
    reader: T,
    logs: Arc<Mutex<VecDeque<ManagedLogLine>>>,
    next_offset: Arc<Mutex<u64>>,
    log_path: PathBuf,
    log_writer: Arc<Mutex<File>>,
    broadcaster: broadcast::Sender<ManagedLogLine>,
) where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(message)) = lines.next_line().await {
        let offset = {
            let mut next = next_offset.lock().await;
            let current = *next;
            *next += 1;
            current
        };

        let log_line = ManagedLogLine {
            offset,
            timestamp: Utc::now(),
            stream: stream_name.clone(),
            message,
        };

        if let Ok(serialized) = serde_json::to_string(&log_line) {
            let _ = append_log_line(&log_path, Arc::clone(&log_writer), &serialized).await;
        }

        let mut log_lines = logs.lock().await;
        log_lines.push_back(log_line.clone());
        while log_lines.len() > MAX_LOG_LINES {
            log_lines.pop_front();
        }

        let _ = broadcaster.send(log_line);
    }
}

async fn append_log_line(
    _log_path: &PathBuf,
    log_writer: Arc<Mutex<File>>,
    serialized: &str,
) -> Result<(), std::io::Error> {
    let mut file = log_writer.lock().await;
    file.write_all(serialized.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

fn process_to_summary(process: &sysinfo::Process, note: Option<String>) -> ProcessSummary {
    ProcessSummary {
        pid: process.pid().as_u32(),
        parent_pid: process.parent().map(|pid| pid.as_u32()),
        instance_id: process_instance_id(process),
        name: process.name().to_string(),
        cpu_percent: process.cpu_usage(),
        memory_bytes: process.memory(),
        virtual_memory_bytes: process.virtual_memory(),
        status: format!("{:?}", process.status()),
        started_at: DateTime::from_timestamp(process.start_time() as i64, 0),
        command_line: process.cmd().join(" "),
        executable_path: process.exe().map(|path| path.display().to_string()),
        note,
    }
}

fn process_instance_id(process: &sysinfo::Process) -> String {
    format!("{}:{}", process.pid().as_u32(), process.start_time())
}
