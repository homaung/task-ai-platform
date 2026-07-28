use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path as FsPath, PathBuf},
    time::{Duration, SystemTime},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post, put},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use db::models::ai_room::{AiRoom, CreateAiRoom};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, sync::Mutex};
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    error::ApiError,
    routes::ssh_hosts::{posix_quote, remote_shell_command, require_registered_alias, run_ssh},
};

const ROOM_DIR: &str = ".ai-room";
const LIBRARY_DIR: &str = "library";
const LEGACY_DECISIONS_FILE: &str = "library/legacy-decisions.md";
const LIBRARY_BASELINE_FILE: &str = ".library-baseline.json";
const SESSION_OVERRIDES_FILE: &str = "session-overrides.json";
const DECISIONS_MANAGED_COMMENT: &str =
    "<!-- Task AI Platform가 안정된 AI 작업 기록에서 확정된 결정을 자동 정리합니다. -->";
const INSTRUCTION_VERSION: i64 = 7;
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;
const CLEAR_SEND_ENV: &str = "SendEnv=-*";
const START_MARKER: &str = "<!-- task-ai-room:start -->";
const END_MARKER: &str = "<!-- task-ai-room:end -->";
const SESSION_COMPLETE_MARKER: &str = "<!-- task-ai-room:complete -->";
const AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(15);
const TASK_SUMMARY_INTERVAL: Duration = Duration::from_secs(45);
const SESSION_STABLE_AFTER: Duration = Duration::from_secs(120);
const TASK_SUMMARY_STATE_FILE: &str = "task-summary-state.json";
const TASK_DASHBOARD_VERSION: u8 = 7;
const DECISION_DASHBOARD_VERSION: u8 = 4;
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_TASK_SUMMARY_MODEL: &str = "qwen3.5:4b";
const MAX_SESSION_PROMPT_BYTES: usize = 96 * 1024;
const MAX_TASKS_PROMPT_BYTES: usize = 32 * 1024;
const MAX_DECISIONS_PROMPT_BYTES: usize = 48 * 1024;
static SYNC_LOCK: Mutex<()> = Mutex::const_new(());
static TASK_SUMMARY_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Debug, Clone, Serialize, TS)]
pub struct AiRoomEndpointState {
    pub configured: bool,
    pub available: bool,
    pub instruction_installed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct AiRoomRecord {
    pub filename: String,
    pub content: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct AiRoomSnapshot {
    pub room: AiRoom,
    pub instruction: String,
    pub context: String,
    pub decisions: String,
    pub tasks: String,
    pub sessions: Vec<AiRoomRecord>,
    pub session_overrides: BTreeMap<String, String>,
    pub library: Vec<AiRoomRecord>,
    pub conflicts: Vec<String>,
    pub local: AiRoomEndpointState,
    pub remote: AiRoomEndpointState,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateAiRoomDocumentRequest {
    pub content: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateAiRoomSessionStatusRequest {
    pub filename: String,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, TS)]
pub struct SyncAiRoomResponse {
    pub copied_to_local: Vec<String>,
    pub copied_to_remote: Vec<String>,
    pub removed_from_remote: Vec<String>,
    pub conflicts: Vec<String>,
    pub snapshot: AiRoomSnapshot,
}

#[derive(Debug, Default)]
struct EndpointFiles {
    files: BTreeMap<String, String>,
    available: bool,
    error: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TaskSummaryState {
    dashboard_hash: Option<String>,
    tasks_hash: Option<String>,
    decisions_source_hash: Option<String>,
    decisions_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskDashboard {
    items: Vec<TaskDashboardItem>,
}

#[derive(Debug, Deserialize)]
struct TaskDashboardItem {
    status: String,
    title: String,
    next: String,
    blocked: String,
}

#[derive(Debug, Deserialize)]
struct DecisionDashboard {
    items: Vec<DecisionDashboardItem>,
}

#[derive(Debug, Deserialize)]
struct DecisionDashboardItem {
    date: String,
    title: String,
    decision: String,
    rationale: String,
    status: String,
    evidence: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

pub async fn list_rooms(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<AiRoom>>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        AiRoom::find_all(&deployment.db().pool).await?,
    )))
}

pub async fn create_room(
    State(deployment): State<DeploymentImpl>,
    Json(mut payload): Json<CreateAiRoom>,
) -> Result<ResponseJson<ApiResponse<AiRoom>>, ApiError> {
    payload.name = payload.name.trim().to_string();
    if payload.name.is_empty() || payload.name.len() > 120 {
        return Err(ApiError::BadRequest(
            "Room name must be between 1 and 120 characters".into(),
        ));
    }

    let local_root = fs::canonicalize(&payload.local_root)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Local root is not accessible: {error}")))?;
    if !fs::metadata(&local_root).await?.is_dir() {
        return Err(ApiError::BadRequest(
            "Local root must be a directory".into(),
        ));
    }
    payload.local_root = normalized_local_path(&local_root);

    match (&payload.ssh_alias, &payload.remote_root) {
        (Some(alias), Some(root)) => {
            require_registered_alias(alias).await?;
            validate_remote_root(root)?;
        }
        (None, None) => {}
        _ => {
            return Err(ApiError::BadRequest(
                "SSH alias and remote root must be configured together".into(),
            ));
        }
    }

    let room = AiRoom::create(&deployment.db().pool, payload).await?;
    Ok(ResponseJson(ApiResponse::success(room)))
}

pub async fn get_room_snapshot(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn initialize_room(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let room = find_room(&deployment, room_id).await?;
    initialize_local(&room).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn prepare_remote_room(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let room = find_room(&deployment, room_id).await?;
    let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) else {
        return Err(ApiError::BadRequest("This room has no SSH server".into()));
    };
    prepare_remote(&room, alias, root).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn update_document(
    State(deployment): State<DeploymentImpl>,
    Path((room_id, kind)): Path<(Uuid, String)>,
    Json(payload): Json<UpdateAiRoomDocumentRequest>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    if payload.content.len() > MAX_DOCUMENT_BYTES {
        return Err(ApiError::PayloadTooLarge);
    }
    if kind != "context" {
        return Err(ApiError::BadRequest(
            "Tasks and decisions are managed automatically from session records".into(),
        ));
    }
    let relative = document_path(&kind)?;
    let room = find_room(&deployment, room_id).await?;
    write_local_file(&room, relative, &payload.content).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn update_session_status(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
    Json(payload): Json<UpdateAiRoomSessionStatusRequest>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    if !payload.filename.starts_with("sessions/")
        || !payload.filename.ends_with(".md")
        || payload.filename.contains(['\\', '\0', '\r', '\n'])
        || payload.filename["sessions/".len()..].contains('/')
    {
        return Err(ApiError::BadRequest("Invalid session filename".into()));
    }
    let status = payload.status.map(|value| value.trim().to_lowercase());
    if status.as_deref().is_some_and(|value| value != "stopped") {
        return Err(ApiError::BadRequest(
            "Session status must be stopped or null".into(),
        ));
    }

    let room = find_room(&deployment, room_id).await?;
    let room_dir = PathBuf::from(&room.local_root).join(ROOM_DIR);
    if fs::metadata(room_dir.join(&payload.filename))
        .await
        .is_err()
    {
        return Err(ApiError::BadRequest("Session record not found".into()));
    }
    let mut overrides = read_session_overrides(&room_dir).await;
    if let Some(status) = status {
        overrides.insert(payload.filename, status);
    } else {
        overrides.remove(&payload.filename);
    }
    write_session_overrides(&room_dir, &overrides).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn update_library_file(
    State(deployment): State<DeploymentImpl>,
    Path((room_id, filename)): Path<(Uuid, String)>,
    Json(payload): Json<UpdateAiRoomDocumentRequest>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    if payload.content.len() > MAX_DOCUMENT_BYTES {
        return Err(ApiError::PayloadTooLarge);
    }
    let relative = library_path(&filename)?;
    let room = find_room(&deployment, room_id).await?;
    write_local_file(&room, &relative, &payload.content).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn delete_library_file(
    State(deployment): State<DeploymentImpl>,
    Path((room_id, filename)): Path<(Uuid, String)>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let relative = library_path(&filename)?;
    let room = find_room(&deployment, room_id).await?;

    if let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) {
        delete_remote_library_file(alias, root, &filename).await?;
    }

    let path = safe_room_path(&room, &relative)?;
    match fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn import_remote_documents(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<SyncAiRoomResponse>>, ApiError> {
    let _guard = SYNC_LOCK.lock().await;
    let room = find_room(&deployment, room_id).await?;
    if room.ssh_alias.is_none() || room.remote_root.is_none() {
        return Err(ApiError::BadRequest("This room has no SSH server".into()));
    }

    let mut local = read_local_files(&room).await;
    let remote = read_remote_files(&room).await;
    if !remote.available {
        return Err(ApiError::BadRequest(
            remote
                .error
                .unwrap_or_else(|| "Server room documents are unavailable".into()),
        ));
    }

    let (copied_to_local, conflicts) =
        merge_remote_library_documents(&room, &mut local.files, &remote.files).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    let snapshot = build_snapshot(room).await;
    Ok(ResponseJson(ApiResponse::success(SyncAiRoomResponse {
        copied_to_local,
        copied_to_remote: Vec::new(),
        removed_from_remote: Vec::new(),
        conflicts,
        snapshot,
    })))
}

pub async fn sync_room(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<SyncAiRoomResponse>>, ApiError> {
    let _guard = SYNC_LOCK.lock().await;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        sync_room_internal(&deployment, room).await?,
    )))
}

async fn sync_room_internal(
    deployment: &DeploymentImpl,
    room: AiRoom,
) -> Result<SyncAiRoomResponse, ApiError> {
    let mut local = read_local_files(&room).await;
    let mut copied_to_local = Vec::new();
    let copied_to_remote = Vec::new();
    let mut removed_from_remote = Vec::new();
    let mut conflicts = Vec::new();

    if let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) {
        let remote = read_remote_files(&room).await;
        if !remote.available {
            return Err(ApiError::BadRequest(
                "Prepare the server before starting an AI session, then sync when it ends".into(),
            ));
        }

        let (checkpoint_copies, checkpoint_conflicts) =
            sync_remote_checkpoints(&room, &mut local, &remote).await?;
        copied_to_local.extend(checkpoint_copies);
        conflicts.extend(checkpoint_conflicts);

        if all_remote_sessions_complete(&remote.files) && conflicts.is_empty() {
            removed_from_remote = clean_remote_room(alias, root, &remote.files).await?;
        } else if !all_remote_sessions_complete(&remote.files) {
            upgrade_remote_instructions(
                &room,
                alias,
                root,
                remote.files.get("ROOM.md").map(String::as_str),
            )
            .await?;
        }
    }

    let room_id = room.id;
    AiRoom::touch(&deployment.db().pool, room_id).await?;
    let room = find_room(deployment, room_id).await?;
    let snapshot = build_snapshot(room).await;
    Ok(SyncAiRoomResponse {
        copied_to_local,
        copied_to_remote,
        removed_from_remote,
        conflicts,
        snapshot,
    })
}

async fn sync_remote_checkpoints(
    room: &AiRoom,
    local: &mut EndpointFiles,
    remote: &EndpointFiles,
) -> Result<(Vec<String>, Vec<String>), ApiError> {
    let mut copied = Vec::new();
    let mut conflicts = Vec::new();

    for (filename, content) in remote
        .files
        .iter()
        .filter(|(name, _)| name.starts_with("sessions/"))
    {
        match local.files.get(filename) {
            None => {
                write_local_file(room, filename, content).await?;
                local.files.insert(filename.clone(), content.clone());
                copied.push(filename.clone());
            }
            Some(local_content) if local_content == content => {}
            Some(local_content) if content.starts_with(local_content) => {
                write_local_file(room, filename, content).await?;
                local.files.insert(filename.clone(), content.clone());
                copied.push(filename.clone());
            }
            Some(local_content) if local_content.starts_with(content) => {}
            Some(local_content)
                if session_status(local_content)
                    .is_some_and(|status| status_is_stopped(&status)) => {}
            Some(_) => conflicts.push(filename.clone()),
        }
    }

    if let (Some(local_context), Some(remote_context)) = (
        local.files.get("context.md"),
        remote.files.get("context.md"),
    ) && local_context != remote_context
    {
        if remote_context.starts_with(local_context) {
            write_local_file(room, "context.md", remote_context).await?;
            local
                .files
                .insert("context.md".into(), remote_context.clone());
            copied.push("context.md".into());
        } else {
            conflicts.push("context.md".into());
        }
    }

    if let (Some(local_decisions), Some(remote_decisions)) = (
        local.files.get("decisions.md"),
        remote.files.get("decisions.md"),
    ) && !local_decisions.contains(DECISIONS_MANAGED_COMMENT)
        && local_decisions != remote_decisions
    {
        if remote_decisions.starts_with(local_decisions)
            || local_decisions.trim()
                == "# Decisions\n\nAppend dated architectural and product decisions here."
        {
            write_local_file(room, "decisions.md", remote_decisions).await?;
            local
                .files
                .insert("decisions.md".into(), remote_decisions.clone());
            copied.push("decisions.md".into());
        } else {
            conflicts.push("decisions.md".into());
        }
    }

    let (library_copies, library_conflicts) =
        merge_remote_library_documents(room, &mut local.files, &remote.files).await?;
    copied.extend(library_copies);
    conflicts.extend(library_conflicts);
    Ok((copied, conflicts))
}

async fn upgrade_remote_instructions(
    room: &AiRoom,
    alias: &str,
    root: &str,
    current_instruction: Option<&str>,
) -> Result<(), ApiError> {
    let instruction = room_instruction(room);
    if current_instruction == Some(instruction.as_str()) {
        return Ok(());
    }
    let manifest = serde_json::json!({
        "room_id": room.id,
        "name": room.name,
        "instruction_version": INSTRUCTION_VERSION,
    });
    let block = managed_agent_block(room);
    let mut files = vec![
        (
            "room.json".into(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ),
        ("ROOM.md".into(), instruction),
    ];
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let existing = read_remote_root_file(alias, root, filename)
            .await
            .unwrap_or_default();
        files.push((filename.into(), upsert_managed_block(&existing, &block)));
    }
    write_remote_files(alias, root, &files).await
}

pub fn spawn_auto_sync(deployment: DeploymentImpl) {
    let summary_deployment = deployment.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TASK_SUMMARY_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            let _guard = TASK_SUMMARY_LOCK.lock().await;
            if let Err(error) = summarize_pending_sessions(&summary_deployment).await {
                tracing::debug!("AI Room local task summarizer is waiting: {error}");
            }
        }
    });

    tokio::spawn(async move {
        match AiRoom::find_all(&deployment.db().pool).await {
            Ok(rooms) => {
                for room in rooms {
                    if let Err(error) = initialize_local(&room).await {
                        tracing::warn!(
                            room_id = %room.id,
                            "AI Room instructions could not be upgraded: {error}"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!("AI Room instruction upgrade could not list rooms: {error}");
            }
        }

        let mut interval = tokio::time::interval(AUTO_SYNC_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;

        loop {
            interval.tick().await;
            let rooms = match AiRoom::find_all(&deployment.db().pool).await {
                Ok(rooms) => rooms,
                Err(error) => {
                    tracing::warn!("AI Room automatic sync could not list rooms: {error}");
                    continue;
                }
            };

            for room in rooms
                .into_iter()
                .filter(|room| room.ssh_alias.is_some() && room.remote_root.is_some())
            {
                let remote = read_remote_files(&room).await;
                if !remote.available {
                    continue;
                }

                let _guard = SYNC_LOCK.lock().await;
                if all_remote_sessions_complete(&remote.files) {
                    match sync_room_internal(&deployment, room).await {
                        Ok(result) if result.conflicts.is_empty() => {
                            tracing::info!(
                                copied = result.copied_to_local.len(),
                                removed = result.removed_from_remote.len(),
                                "AI Room automatically synchronized and cleaned the server"
                            );
                        }
                        Ok(result) => {
                            tracing::warn!(
                                conflicts = result.conflicts.len(),
                                "AI Room automatic sync preserved conflicting server records"
                            );
                        }
                        Err(error) => {
                            tracing::warn!("AI Room automatic sync failed: {error}");
                        }
                    }
                    continue;
                }

                let mut local = read_local_files(&room).await;
                match sync_remote_checkpoints(&room, &mut local, &remote).await {
                    Ok((copied, conflicts)) => {
                        if let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root)
                            && let Err(error) = upgrade_remote_instructions(
                                &room,
                                alias,
                                root,
                                remote.files.get("ROOM.md").map(String::as_str),
                            )
                            .await
                        {
                            tracing::warn!(
                                "AI Room could not upgrade active server instructions: {error}"
                            );
                        }
                        if !copied.is_empty() || !conflicts.is_empty() {
                            tracing::info!(
                                copied = copied.len(),
                                conflicts = conflicts.len(),
                                "AI Room mirrored active server checkpoints without cleanup"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!("AI Room active checkpoint sync failed: {error}");
                    }
                }
            }
        }
    });
}

pub async fn delete_room(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    if !AiRoom::delete(&deployment.db().pool, room_id).await? {
        return Err(ApiError::BadRequest("Room not found".into()));
    }
    Ok(ResponseJson(ApiResponse::success(())))
}

async fn find_room(deployment: &DeploymentImpl, id: Uuid) -> Result<AiRoom, ApiError> {
    AiRoom::find_by_id(&deployment.db().pool, id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Room not found".into()))
}

fn validate_remote_root(root: &str) -> Result<(), ApiError> {
    if root.is_empty() || root.len() > 4096 || root.contains(['\r', '\n', '\0']) {
        return Err(ApiError::BadRequest("Invalid remote root".into()));
    }
    Ok(())
}

fn normalized_local_path(path: &FsPath) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    value.into_owned()
}

fn room_instruction(room: &AiRoom) -> String {
    let instruction = format!(
        "# AI Room: {name}\n\nRoom ID: `{id}`\nInstruction version: {version}\n\n## Required session workflow\n\n1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, every relevant file in `.ai-room/library/`, every additional Markdown instruction directly under `.ai-room/`, and the newest files in `.ai-room/sessions/`.\n2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.\n3. Record the goal, assumptions, important commands, files changed, verification results, candidate decisions, blockers, and concrete next steps. Write the first checkpoint immediately. Update it after every meaningful work unit and never allow more than 10 minutes of active work without a checkpoint. A chat may remain open for months; checkpoints are work-state records, not chat endings. If the user pauses, stops, or cancels work while you can still write, immediately record the status as Stopped or Cancelled.\n4. When the user asks you to remember a reusable method, rule, convention, checklist, prompt, or operating procedure, create or update one focused Markdown file in `.ai-room/library/`. Use a descriptive filename ending in `.md`, keep one topic per file, and make it understandable without chat history. Do not use the library for transient session notes.\n5. Do not edit `tasks.md` or `decisions.md`. Task AI Platform reads stable session checkpoints together and locally rebuilds both documents. State results, approval status, next actions, blockers, and whether a candidate decision was actually approved explicitly in the session file.\n6. Treat `context.md` as owner-authored project context. Read it but do not edit it unless the user explicitly asks you to change that document.\n7. Never store secrets, tokens, private keys, raw credentials, personal data, or generated binaries in room files.\n8. Before ending the entire work record, make the session file sufficient for another Claude or Codex session to continue without relying on chat history. After all other writes are finished, add `{complete_marker}` as the final line. The completion marker is only for safe server cleanup; local task and decision updates use stable intermediate checkpoints after two quiet minutes.\n\n## Server privacy\n\nWhile work is active, Task AI Platform copies changing server session checkpoints to local storage without deleting the server files. Once every remote session is complete and the merge is conflict-free, it removes the temporary server room. Task and decision summarization uses only the local Ollama service; session contents are not sent to a cloud model.\n\n## Room endpoints\n\n- Local root: `{local}`\n- Remote root: `{remote}`\n\nThe Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.\n",
        name = room.name,
        id = room.id,
        version = INSTRUCTION_VERSION,
        complete_marker = SESSION_COMPLETE_MARKER,
        local = room.local_root,
        remote = room
            .ssh_alias
            .as_ref()
            .zip(room.remote_root.as_ref())
            .map(|(alias, root)| format!("{alias}:{root}"))
            .unwrap_or_else(|| "not configured".into()),
    );
    format!(
        "{instruction}\n## Record language\n\n- Session checkpoint files may use whichever language lets the active AI preserve technical meaning and handoff context most accurately; they do not need to be Korean.\n- `decisions.md` is shared by the owner and every AI. Task AI Platform renders its explanatory text in Korean and translates session content when necessary. Keep code identifiers, file paths, and product names unchanged when translation would damage their meaning.\n"
    )
}

fn managed_agent_block(room: &AiRoom) -> String {
    format!(
        "{START_MARKER}\n## Shared AI Room\n\nThis project belongs to AI Room `{}` (`{}`). Before every task, read `.ai-room/ROOM.md` and follow its session recording workflow. Keep all durable handoff records under `.ai-room`; never put secrets there.\n{END_MARKER}",
        room.name, room.id
    )
}

fn upsert_managed_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end_start)) = (existing.find(START_MARKER), existing.find(END_MARKER))
        && end_start >= start
    {
        let end = end_start + END_MARKER.len();
        return format!("{}{}{}", &existing[..start], block, &existing[end..]);
    }
    if existing.trim().is_empty() {
        format!("{block}\n")
    } else {
        format!("{}\n\n{block}\n", existing.trim_end())
    }
}

fn initial_files(room: &AiRoom) -> Vec<(String, String)> {
    let manifest = serde_json::json!({
        "room_id": room.id,
        "name": room.name,
        "instruction_version": INSTRUCTION_VERSION,
    });
    vec![
        (
            "room.json".into(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ),
        ("ROOM.md".into(), room_instruction(room)),
        (
            "context.md".into(),
            format!(
                "# Context\n\n프로젝트 소유자가 직접 관리하는 {}의 지속적인 맥락입니다.\n",
                room.name
            ),
        ),
        (
            "decisions.md".into(),
            format!(
                "# Decisions\n\n{DECISIONS_MANAGED_COMMENT}\n\n## 현재 결정\n\n- 아직 기록된 확정 결정이 없습니다.\n\n## 변경·폐기된 결정\n\n- 아직 기록된 변경·폐기 결정이 없습니다.\n"
            ),
        ),
        (
            "tasks.md".into(),
            "# Tasks\n\n<!-- Task AI Platform가 안정된 AI 작업 기록을 바탕으로 이 대시보드를 자동 갱신합니다. -->\n\n## 진행 중\n\n- 현재 진행 중인 작업이 없습니다.\n\n## 완료\n\n- 아직 기록된 완료 항목이 없습니다.\n"
                .into(),
        ),
    ]
}

fn all_remote_sessions_complete(files: &BTreeMap<String, String>) -> bool {
    let sessions = files
        .iter()
        .filter(|(name, _)| name.starts_with("sessions/"))
        .collect::<Vec<_>>();
    !sessions.is_empty()
        && sessions
            .iter()
            .all(|(_, content)| session_is_complete(content))
}

fn session_is_complete(content: &str) -> bool {
    content
        .lines()
        .next_back()
        .is_some_and(|line| line.trim() == SESSION_COMPLETE_MARKER)
}

fn session_is_ready_for_summary(
    content: &str,
    modified: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    session_is_complete(content)
        || modified
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|idle| idle >= SESSION_STABLE_AFTER)
}

#[derive(Debug, PartialEq, Eq)]
enum LibraryMergeAction {
    Same,
    AcceptRemote,
    KeepLocal,
    Conflict,
}

fn library_merge_action(
    local: &str,
    remote: &str,
    baseline_hash: Option<&str>,
) -> LibraryMergeAction {
    if local == remote {
        return LibraryMergeAction::Same;
    }
    match baseline_hash {
        Some(baseline) if content_hash(local) == baseline => LibraryMergeAction::AcceptRemote,
        Some(baseline) if content_hash(remote) == baseline => LibraryMergeAction::KeepLocal,
        _ => LibraryMergeAction::Conflict,
    }
}

fn library_baseline(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    files
        .get(LIBRARY_BASELINE_FILE)
        .and_then(|content| serde_json::from_str(content).ok())
        .unwrap_or_default()
}

async fn merge_remote_library_documents(
    room: &AiRoom,
    local_files: &mut BTreeMap<String, String>,
    remote_files: &BTreeMap<String, String>,
) -> Result<(Vec<String>, Vec<String>), ApiError> {
    let baseline = library_baseline(remote_files);
    let remote_documents = remote_library_documents(remote_files);
    let mut copied = Vec::new();
    let mut conflicts = Vec::new();

    for (filename, remote_content) in &remote_documents {
        match local_files.get(filename) {
            None => {
                write_local_file(room, &filename, remote_content).await?;
                local_files.insert(filename.clone(), remote_content.clone());
                copied.push(filename.clone());
            }
            Some(local_content) => {
                match library_merge_action(
                    local_content,
                    remote_content,
                    baseline.get(filename).map(String::as_str),
                ) {
                    LibraryMergeAction::Same | LibraryMergeAction::KeepLocal => {}
                    LibraryMergeAction::AcceptRemote => {
                        write_local_file(room, &filename, remote_content).await?;
                        local_files.insert(filename.clone(), remote_content.clone());
                        copied.push(filename.clone());
                    }
                    LibraryMergeAction::Conflict => conflicts.push(filename.clone()),
                }
            }
        }
    }

    Ok((copied, conflicts))
}

fn ensure_task_update_section(content: &str) -> String {
    let dashboard_comment =
        "<!-- Task AI Platform가 안정된 AI 작업 기록을 바탕으로 이 대시보드를 자동 갱신합니다. -->";
    let normalized = content
        .replace(
            "<!-- The local task summarizer appends one validated line per completed session. -->",
            dashboard_comment,
        )
        .replace(
            "<!-- AI agents append exactly one concise line per session below. -->",
            dashboard_comment,
        );
    if normalized.contains("## 진행 중") && normalized.contains("## 완료") {
        normalized
    } else {
        format!(
            "{}\n\n{dashboard_comment}\n\n## 진행 중\n\n- 현재 진행 중인 작업이 없습니다.\n\n## 완료\n\n- 아직 기록된 완료 항목이 없습니다.\n",
            normalized.trim_end()
        )
    }
}

async fn summarize_pending_sessions(deployment: &DeploymentImpl) -> Result<(), String> {
    let rooms = AiRoom::find_all(&deployment.db().pool)
        .await
        .map_err(|error| error.to_string())?;
    let mut first_error = None;

    for room in rooms {
        if let Err(error) = summarize_next_session(&room).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }

    first_error.map_or(Ok(()), Err)
}

async fn summarize_next_session(room: &AiRoom) -> Result<(), String> {
    let room_dir = PathBuf::from(&room.local_root).join(ROOM_DIR);
    let tasks_path = room_dir.join("tasks.md");
    let decisions_path = room_dir.join("decisions.md");
    let initial_tasks = fs::read_to_string(&tasks_path)
        .await
        .map_err(|error| format!("{} has no readable task list: {error}", room.name))?;
    let initial_decisions = fs::read_to_string(&decisions_path)
        .await
        .map_err(|error| format!("{} has no readable decision log: {error}", room.name))?;
    let legacy_decisions = fs::read_to_string(room_dir.join(LEGACY_DECISIONS_FILE))
        .await
        .unwrap_or_default();
    let decision_sources = if legacy_decisions.is_empty() {
        initial_decisions.clone()
    } else {
        format!(
            "===== {} =====\n{}\n\n===== decisions.md =====\n{}",
            LEGACY_DECISIONS_FILE, legacy_decisions, initial_decisions
        )
    };
    let mut state = read_task_summary_state(&room_dir).await;
    let overrides = read_session_overrides(&room_dir).await;
    let mut sessions = Vec::new();
    let mut entries = match fs::read_dir(room_dir.join("sessions")).await {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(entry.path()).await
            && content.len() <= MAX_DOCUMENT_BYTES
        {
            let modified = entry
                .metadata()
                .await
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            if session_is_ready_for_summary(&content, modified, SystemTime::now()) {
                sessions.push((format!("sessions/{name}"), content));
            }
        }
    }
    sessions.sort_by(|left, right| left.0.cmp(&right.0));

    if sessions.is_empty() {
        return Ok(());
    }

    let overrides_json = serde_json::to_string(&overrides).unwrap_or_default();
    let dashboard_hash = format!(
        "{}:{}",
        session_collection_hash(&sessions),
        content_hash(&overrides_json)
    );
    let decisions_source_hash =
        versioned_session_collection_hash(DECISION_DASHBOARD_VERSION, &sessions, &legacy_decisions);
    let initial_tasks_hash = content_hash(&initial_tasks);
    let initial_decisions_hash = content_hash(&initial_decisions);
    let tasks_current = state.dashboard_hash.as_ref() == Some(&dashboard_hash)
        && state.tasks_hash.as_ref() == Some(&initial_tasks_hash);
    let decisions_current = state.decisions_source_hash.as_ref() == Some(&decisions_source_hash)
        && state.decisions_hash.as_ref() == Some(&initial_decisions_hash);
    if tasks_current && decisions_current {
        return Ok(());
    }

    if !tasks_current {
        let dashboard = request_local_task_dashboard(&sessions, &initial_tasks).await?;
        let dashboard = remove_stopped_sessions(dashboard, &sessions, &overrides);
        let dashboard = ensure_latest_next_action(dashboard, &sessions, &overrides);
        let content = render_task_dashboard(dashboard)?;
        let tasks_hash = content_hash(&content);
        fs::write(&tasks_path, content)
            .await
            .map_err(|error| format!("Unable to update {} tasks: {error}", room.name))?;
        state.dashboard_hash = Some(dashboard_hash);
        state.tasks_hash = Some(tasks_hash);
        write_task_summary_state(&room_dir, &state).await?;
    }

    if !decisions_current {
        let dashboard = request_local_decision_dashboard(&sessions, &decision_sources).await?;
        let dashboard = augment_explicit_decisions(dashboard, &sessions);
        let content = render_decision_dashboard(
            dashboard,
            &sessions,
            (!legacy_decisions.is_empty())
                .then_some((LEGACY_DECISIONS_FILE, legacy_decisions.as_str())),
        )?;
        let decisions_hash = content_hash(&content);
        fs::write(&decisions_path, content)
            .await
            .map_err(|error| format!("Unable to update {} decisions: {error}", room.name))?;
        state.decisions_source_hash = Some(decisions_source_hash);
        state.decisions_hash = Some(decisions_hash);
        write_task_summary_state(&room_dir, &state).await?;
    }
    tracing::info!(
        room_id = %room.id,
        sessions = sessions.len(),
        "AI Room rebuilt local task and decision dashboards"
    );
    Ok(())
}

async fn read_task_summary_state(room_dir: &FsPath) -> TaskSummaryState {
    let Ok(content) = fs::read_to_string(room_dir.join(TASK_SUMMARY_STATE_FILE)).await else {
        return TaskSummaryState::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

async fn write_task_summary_state(
    room_dir: &FsPath,
    state: &TaskSummaryState,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(room_dir.join(TASK_SUMMARY_STATE_FILE), content)
        .await
        .map_err(|error| format!("Unable to save local task summary state: {error}"))
}

async fn read_session_overrides(room_dir: &FsPath) -> BTreeMap<String, String> {
    let Ok(content) = fs::read_to_string(room_dir.join(SESSION_OVERRIDES_FILE)).await else {
        return BTreeMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

async fn write_session_overrides(
    room_dir: &FsPath,
    overrides: &BTreeMap<String, String>,
) -> Result<(), ApiError> {
    let content = serde_json::to_string_pretty(overrides)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    fs::write(room_dir.join(SESSION_OVERRIDES_FILE), content).await?;
    Ok(())
}

fn content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn session_collection_hash(sessions: &[(String, String)]) -> String {
    versioned_session_collection_hash(TASK_DASHBOARD_VERSION, sessions, "")
}

fn versioned_session_collection_hash(
    version: u8,
    sessions: &[(String, String)],
    seed: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update([version]);
    hasher.update(seed.as_bytes());
    hasher.update([0xfe]);
    for (filename, content) in sessions {
        hasher.update(filename.as_bytes());
        hasher.update([0]);
        hasher.update(content.as_bytes());
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
}

async fn request_local_task_dashboard(
    sessions: &[(String, String)],
    tasks: &str,
) -> Result<TaskDashboard, String> {
    let base_url = env::var("TASK_AI_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_URL.into());
    let model =
        env::var("TASK_AI_SUMMARY_MODEL").unwrap_or_else(|_| DEFAULT_TASK_SUMMARY_MODEL.into());
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "maxItems": 18,
                "items": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["active", "blocked", "completed"]
                        },
                        "title": { "type": "string" },
                        "next": { "type": "string" },
                        "blocked": { "type": "string" }
                    },
                    "required": ["status", "title", "next", "blocked"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    });
    let mut records = String::new();
    for (filename, content) in sessions {
        records.push_str("\n\n===== ");
        records.push_str(filename);
        records.push_str(" =====\n");
        records.push_str(&bounded_text(content, 12 * 1024));
    }
    let prompt = format!(
        "프로젝트의 현재 작업 대시보드를 시간순 AI 작업 기록 전체로부터 다시 작성하라. 반드시 요청된 JSON 스키마만 반환하라.\n\
         규칙:\n\
         - 뒤의 기록이 앞의 기록보다 최신이며, 서로 충돌하면 최신 기록을 따른다.\n\
         - 세션마다 항목을 만들지 말고 같은 목표와 후속 작업은 하나로 병합한다.\n\
         - 이미 해결되었거나 최신 기록에서 무효가 된 차단 사유와 다음 작업은 제거한다.\n\
         - active/blocked에는 지금 실제로 할 수 있는 구체적인 작업만 최대 8개 넣는다.\n\
         - completed에는 의미 있는 완료 결과만 최대 10개 넣고 단순 대기나 반복 보고는 제외한다.\n\
         - 완료된 목표와 같은 진행 항목은 completed만 남긴다.\n\
         - stopped, cancelled, 중단, 취소된 작업은 active와 completed 어디에도 넣지 않는다.\n\
         - 원문이 영어여도 title, next, blocked의 모든 출력은 반드시 자연스러운 한국어로 번역한다. 해당 내용이 없으면 none을 사용한다.\n\
         - Status가 In progress, awaiting, pending, blocked인 기록을 completed로 분류하지 않는다.\n\
         - 필드 안에 마크다운, 줄바꿈, 파이프 문자를 넣지 않는다.\n\
         기존 작업 목록은 오래된 초안일 수 있다. 작업 기록을 사실의 원천으로 삼는다.\n\n\
         기존 작업 목록:\n{}\n\n\
         시간순 작업 기록:\n{}",
        bounded_text(tasks, MAX_TASKS_PROMPT_BYTES),
        bounded_text(&records, MAX_SESSION_PROMPT_BYTES)
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{}/api/chat", base_url.trim_end_matches('/')))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{ "role": "user", "content": prompt }],
            "format": schema,
            "stream": false,
            "think": false,
            "keep_alive": "5m",
            "options": { "temperature": 0 }
        }))
        .send()
        .await
        .map_err(|error| format!("Local Ollama is unavailable: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Unable to read Ollama response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Ollama rejected task summarization ({status}): {}",
            bounded_text(&body, 300)
        ));
    }
    let response: OllamaChatResponse =
        serde_json::from_str(&body).map_err(|error| format!("Invalid Ollama response: {error}"))?;
    serde_json::from_str(response.message.content.trim())
        .map_err(|error| format!("Ollama returned invalid task dashboard JSON: {error}"))
}

async fn request_local_decision_dashboard(
    sessions: &[(String, String)],
    decisions: &str,
) -> Result<DecisionDashboard, String> {
    let base_url = env::var("TASK_AI_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_URL.into());
    let model =
        env::var("TASK_AI_SUMMARY_MODEL").unwrap_or_else(|_| DEFAULT_TASK_SUMMARY_MODEL.into());
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "maxItems": 30,
                "items": {
                    "type": "object",
                    "properties": {
                        "date": { "type": "string" },
                        "title": { "type": "string" },
                        "decision": { "type": "string" },
                        "rationale": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["current", "superseded"]
                        },
                        "evidence": {
                            "type": "array",
                            "maxItems": 4,
                            "items": { "type": "string" }
                        }
                    },
                    "required": [
                        "date",
                        "title",
                        "decision",
                        "rationale",
                        "status",
                        "evidence"
                    ],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    });
    let mut records = String::new();
    for (filename, content) in sessions {
        records.push_str("\n\n===== ");
        records.push_str(filename);
        records.push_str(" =====\n");
        records.push_str(&bounded_text(content, 12 * 1024));
    }
    let prompt = format!(
        "프로젝트의 결정 기록을 시간순 AI 작업 기록 전체로부터 다시 작성하라. 반드시 요청된 JSON 스키마만 반환하라.\n\
         규칙:\n\
         - 사용자가 승인했거나 실제 구현·검증 결과로 확정된 지속적인 기술·제품·운영 결정만 포함한다.\n\
         - 제안, 선택 대기, 미승인 계획, 단순 작업 결과, 다음 할 일은 결정으로 만들지 않는다.\n\
         - 뒤의 기록이 앞의 기록보다 최신이다. 같은 주제는 병합하고 변경된 옛 결정은 superseded로 표시한다.\n\
         - 중단된 세션에서도 이미 명시적으로 확정된 결정만 보존한다. 중단 사실 자체는 결정이 아니다.\n\
         - 기존 결정 기록의 확정 내용은 세션과 충돌하지 않는 한 보존하되 중복은 제거한다.\n\
         - date는 YYYY-MM-DD로 작성한다. title, decision, rationale은 원문 언어와 관계없이 반드시 자연스러운 한국어로 번역·작성한다.\n\
         - 코드 식별자, 파일 경로, 제품명처럼 번역하면 의미가 훼손되는 고유명사만 원문 표기를 허용한다. 설명 문장 전체를 영어로 남기지 않는다.\n\
         - evidence에는 반드시 아래에 제공된 sessions/... 파일명 또는 library/legacy-decisions.md를 넣는다. 근거가 없는 항목은 만들지 않는다.\n\
         - 필드 안에 마크다운, 줄바꿈, 파이프 문자를 넣지 않는다.\n\n\
         기존 결정 기록:\n{}\n\n\
         시간순 작업 기록:\n{}",
        bounded_text(decisions, MAX_DECISIONS_PROMPT_BYTES),
        bounded_text(&records, MAX_SESSION_PROMPT_BYTES)
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{}/api/chat", base_url.trim_end_matches('/')))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{ "role": "user", "content": prompt }],
            "format": schema,
            "stream": false,
            "think": false,
            "keep_alive": "5m",
            "options": { "temperature": 0 }
        }))
        .send()
        .await
        .map_err(|error| format!("Local Ollama is unavailable: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Unable to read Ollama response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Ollama rejected decision summarization ({status}): {}",
            bounded_text(&body, 300)
        ));
    }
    let response: OllamaChatResponse =
        serde_json::from_str(&body).map_err(|error| format!("Invalid Ollama response: {error}"))?;
    serde_json::from_str(response.message.content.trim())
        .map_err(|error| format!("Ollama returned invalid decision dashboard JSON: {error}"))
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let head_bytes = max_bytes * 3 / 4;
    let tail_bytes = max_bytes - head_bytes;
    let mut head_end = head_bytes;
    while !value.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = value.len() - tail_bytes;
    while !value.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!(
        "{}\n...[truncated]...\n{}",
        &value[..head_end],
        &value[tail_start..]
    )
}

fn clean_summary_field(value: &str, max_chars: usize, fallback: &str) -> String {
    let cleaned = value
        .replace('|', "/")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let result = if cleaned.is_empty() {
        fallback
    } else {
        &cleaned
    };
    result.chars().take(max_chars).collect()
}

fn is_none_value(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "" | "none" | "n/a" | "없음" | "해당 없음"
    )
}

fn contains_hangul(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '\u{AC00}'..='\u{D7A3}'))
}

fn task_title_key(value: &str) -> String {
    clean_summary_field(value, 180, "").to_lowercase()
}

fn session_title(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# Session:")
            .map(|title| clean_summary_field(title, 180, ""))
            .filter(|title| !title.is_empty())
    })
}

fn session_status(content: &str) -> Option<String> {
    let mut lines = content.lines();
    while let Some(line) = lines.next() {
        if line.trim().eq_ignore_ascii_case("## Status") {
            return lines
                .find(|candidate| !candidate.trim().is_empty())
                .map(|status| {
                    clean_summary_field(status.trim().trim_start_matches('-'), 100, "")
                        .to_lowercase()
                });
        }
    }
    None
}

fn status_is_stopped(status: &str) -> bool {
    ["stopped", "cancelled", "canceled", "중단", "취소"]
        .iter()
        .any(|marker| status.contains(marker))
}

fn same_task_title(left: &str, right: &str) -> bool {
    let left = task_title_key(left);
    let right = task_title_key(right);
    left == right
        || (left.chars().count().min(right.chars().count()) >= 12
            && (left.starts_with(&right) || right.starts_with(&left)))
}

fn remove_stopped_sessions(
    mut dashboard: TaskDashboard,
    sessions: &[(String, String)],
    overrides: &BTreeMap<String, String>,
) -> TaskDashboard {
    let mut stopped_titles = sessions
        .iter()
        .filter_map(|(_, content)| {
            let status = session_status(content)?;
            status_is_stopped(&status)
                .then(|| session_title(content))
                .flatten()
        })
        .collect::<Vec<_>>();
    stopped_titles.extend(sessions.iter().filter_map(|(filename, content)| {
        overrides
            .get(filename)
            .is_some_and(|status| status == "stopped")
            .then(|| session_title(content))
            .flatten()
    }));
    dashboard.items.retain(|item| {
        !stopped_titles
            .iter()
            .any(|title| same_task_title(&item.title, title))
    });
    dashboard
}

fn extract_next_action(content: &str) -> Option<String> {
    for line in content.lines().rev() {
        let candidate = line
            .trim()
            .trim_start_matches(['-', '*'])
            .trim()
            .trim_start_matches("**")
            .trim_end_matches("**");
        for marker in ["후속=", "후속:", "Next:", "다음:"] {
            if let Some((_, action)) = candidate.split_once(marker) {
                let action = clean_summary_field(action, 180, "");
                if !action.is_empty() && !is_none_value(&action) {
                    return Some(action);
                }
            }
        }
    }

    let lines = content.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate().rev() {
        if line.trim().eq_ignore_ascii_case("## Next") {
            return lines[index + 1..].iter().find_map(|line| {
                let action =
                    clean_summary_field(line.trim().trim_start_matches(['-', '*']).trim(), 180, "");
                (!action.is_empty() && !is_none_value(&action)).then_some(action)
            });
        }
    }
    None
}

fn ensure_latest_next_action(
    mut dashboard: TaskDashboard,
    sessions: &[(String, String)],
    overrides: &BTreeMap<String, String>,
) -> TaskDashboard {
    if dashboard
        .items
        .iter()
        .any(|item| matches!(item.status.to_lowercase().as_str(), "active" | "blocked"))
    {
        return dashboard;
    }
    let Some((filename, content)) = sessions.last() else {
        return dashboard;
    };
    if overrides
        .get(filename)
        .is_some_and(|status| status == "stopped")
        || session_status(content).is_some_and(|status| status_is_stopped(&status))
    {
        return dashboard;
    }
    let Some(action) = extract_next_action(content) else {
        return dashboard;
    };
    dashboard.items.insert(
        0,
        TaskDashboardItem {
            status: "active".into(),
            title: action,
            next: "최신 체크포인트에 기록된 후속 작업을 수행한다".into(),
            blocked: "none".into(),
        },
    );
    dashboard
}

fn render_task_dashboard(dashboard: TaskDashboard) -> Result<String, String> {
    let mut completed = Vec::new();
    let mut completed_keys = BTreeSet::new();
    for item in &dashboard.items {
        if item.status.eq_ignore_ascii_case("completed") {
            let title = clean_summary_field(&item.title, 180, "");
            let key = task_title_key(&title);
            if !key.is_empty() && completed_keys.insert(key) && completed.len() < 10 {
                completed.push(title);
            }
        }
    }

    let mut active = Vec::new();
    let mut active_keys = BTreeSet::new();
    for item in dashboard.items {
        if !matches!(item.status.to_lowercase().as_str(), "active" | "blocked") {
            continue;
        }
        let title = clean_summary_field(&item.title, 180, "");
        let key = task_title_key(&title);
        if key.is_empty()
            || completed_keys.contains(&key)
            || !active_keys.insert(key)
            || active.len() >= 8
        {
            continue;
        }
        let next = clean_summary_field(&item.next, 140, "none");
        let blocked = clean_summary_field(&item.blocked, 100, "none");
        active.push((title, next, blocked));
    }

    if active.is_empty() && completed.is_empty() {
        return Err("Local model returned an empty task dashboard".into());
    }

    let mut output = String::from(
        "# Tasks\n\n<!-- Task AI Platform가 안정된 AI 작업 기록을 바탕으로 이 대시보드를 자동 갱신합니다. -->\n\n## 진행 중\n\n",
    );
    if active.is_empty() {
        output.push_str("- 현재 진행 중인 작업이 없습니다.\n");
    } else {
        for (title, next, blocked) in active {
            output.push_str(&format!("- [ ] {title}\n"));
            if !is_none_value(&next) {
                output.push_str(&format!("  - 다음: {next}\n"));
            }
            if !is_none_value(&blocked) {
                output.push_str(&format!("  - 차단: {blocked}\n"));
            }
        }
    }

    output.push_str("\n## 완료\n\n");
    if completed.is_empty() {
        output.push_str("- 아직 기록된 완료 항목이 없습니다.\n");
    } else {
        for title in completed {
            output.push_str(&format!("- [x] {title}\n"));
        }
    }
    Ok(output)
}

fn valid_decision_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, value)| matches!(index, 4 | 7) || value.is_ascii_digit())
}

fn augment_explicit_decisions(
    mut dashboard: DecisionDashboard,
    sessions: &[(String, String)],
) -> DecisionDashboard {
    let mut seen = dashboard
        .items
        .iter()
        .map(|item| clean_summary_field(&item.decision, 360, "").to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut added = 0;

    for (filename, content) in sessions.iter().rev() {
        let title = session_title(content).unwrap_or_else(|| "세션 결정".into());
        for line in content.lines() {
            let normalized = line.trim().trim_start_matches(['-', '*']).trim();
            let lowered = normalized.to_lowercase();
            if ["미승인", "제안", "승인 시", "candidate"]
                .iter()
                .any(|marker| lowered.contains(marker))
            {
                continue;
            }
            let extracted = normalized
                .split_once("결정:")
                .map(|(_, decision)| decision)
                .or_else(|| {
                    normalized.contains("→ 폐기").then(|| {
                        normalized
                            .split_once("판정:")
                            .map_or(normalized, |(_, v)| v)
                    })
                });
            let Some(extracted) = extracted else {
                continue;
            };
            let decision = clean_summary_field(extracted.trim_matches('*'), 360, "");
            let key = decision.to_lowercase();
            if decision.chars().count() < 6 || !contains_hangul(&decision) || !seen.insert(key) {
                continue;
            }
            dashboard.items.push(DecisionDashboardItem {
                date: session_date(filename).unwrap_or_else(|| "날짜 미상".into()),
                title: format!("{} — 명시적 결정", clean_summary_field(&title, 110, "")),
                decision,
                rationale: "세션 체크포인트에서 결정으로 명시됨".into(),
                status: "current".into(),
                evidence: vec![filename.clone()],
            });
            added += 1;
            if added >= 30 {
                return dashboard;
            }
        }
    }
    dashboard
}

fn decision_is_durable(item: &DecisionDashboardItem) -> bool {
    let combined = format!("{} {} {}", item.title, item.decision, item.rationale).to_lowercase();
    [
        "채택",
        "사용한다",
        "사용하지",
        "해야",
        "않는다",
        "기본",
        "유지",
        "전환",
        "금지",
        "규칙",
        "구조",
        "기준",
        "방식",
        "정의",
        "정책",
        "폐기",
        "접음",
        "접는다",
        "adopt",
        "must",
        "default",
        "do not",
        "policy",
        "architecture",
    ]
    .iter()
    .any(|marker| combined.contains(marker))
}

fn session_date(filename: &str) -> Option<String> {
    let stem = filename.strip_prefix("sessions/")?;
    let raw = stem.get(..8)?;
    raw.chars()
        .all(|value| value.is_ascii_digit())
        .then(|| format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
}

fn infer_decision_evidence(
    title: &str,
    decision: &str,
    sessions: &[(String, String)],
) -> Option<String> {
    let query = format!("{} {}", title.to_lowercase(), decision.to_lowercase());
    let tokens = query
        .split(|value: char| !value.is_alphanumeric())
        .filter(|token| token.chars().count() >= 4)
        .filter(|token| {
            !matches!(
                *token,
                "결정" | "결과" | "실행" | "완료" | "사용자" | "프로젝트"
            )
        })
        .collect::<BTreeSet<_>>();
    let mut best = None;
    let mut best_score = 0;
    for (filename, content) in sessions {
        let content = content.to_lowercase();
        let score = tokens
            .iter()
            .filter(|token| content.contains(**token))
            .count();
        if score > 0 && score >= best_score {
            best = Some(filename.clone());
            best_score = score;
        }
    }
    best
}

fn legacy_decision_dates(content: &str) -> BTreeSet<String> {
    content
        .lines()
        .filter_map(|line| {
            let candidate = line.trim().trim_start_matches('#').trim();
            let date = candidate.split_whitespace().next()?;
            valid_decision_date(date).then(|| date.to_string())
        })
        .collect()
}

fn render_decision_dashboard(
    dashboard: DecisionDashboard,
    sessions: &[(String, String)],
    fallback_evidence: Option<(&str, &str)>,
) -> Result<String, String> {
    let mut valid_evidence = sessions
        .iter()
        .map(|(filename, _)| filename.as_str())
        .collect::<BTreeSet<_>>();
    if let Some((filename, _)) = fallback_evidence {
        valid_evidence.insert(filename);
    }
    let allowed_legacy_dates = fallback_evidence
        .map(|(_, content)| legacy_decision_dates(content))
        .unwrap_or_default();
    let mut current = Vec::new();
    let mut superseded = Vec::new();
    let mut seen = BTreeSet::new();

    for item in dashboard.items {
        if !decision_is_durable(&item) {
            continue;
        }
        let title = clean_summary_field(&item.title, 140, "");
        let decision = clean_summary_field(&item.decision, 360, "");
        if title.is_empty()
            || decision.is_empty()
            || !contains_hangul(&title)
            || !contains_hangul(&decision)
        {
            continue;
        }
        let mut evidence = item
            .evidence
            .iter()
            .filter_map(|filename| {
                if valid_evidence.contains(filename.as_str()) {
                    return Some(filename.clone());
                }
                let basename = filename.rsplit('/').next()?;
                valid_evidence
                    .iter()
                    .find(|candidate| candidate.rsplit('/').next() == Some(basename))
                    .map(|candidate| (*candidate).to_string())
            })
            .take(4)
            .collect::<Vec<_>>();
        if evidence.is_empty()
            && let Some(filename) = infer_decision_evidence(&title, &decision, sessions)
        {
            evidence.push(filename);
        }
        if evidence.is_empty()
            && let Some((filename, _)) = fallback_evidence
        {
            evidence.push(filename.to_string());
        }
        if evidence.is_empty() {
            continue;
        }
        let key = format!(
            "{}:{}",
            task_title_key(&title),
            clean_summary_field(&decision, 360, "").to_lowercase()
        );
        if !seen.insert(key) {
            continue;
        }
        let date = evidence
            .iter()
            .filter_map(|filename| session_date(filename))
            .max()
            .or_else(|| {
                allowed_legacy_dates
                    .contains(item.date.trim())
                    .then(|| item.date.trim().to_string())
            })
            .unwrap_or_else(|| "날짜 미상".into());
        let mut rationale = clean_summary_field(&item.rationale, 280, "none");
        if !is_none_value(&rationale) && !contains_hangul(&rationale) {
            rationale = "none".into();
        }
        let entry = (date, title, decision, rationale, evidence);
        if item.status.eq_ignore_ascii_case("superseded") {
            superseded.push(entry);
        } else if item.status.eq_ignore_ascii_case("current") {
            current.push(entry);
        }
    }

    if current.is_empty() && superseded.is_empty() && fallback_evidence.is_some() {
        return Err("Local model returned no usable legacy decisions".into());
    }

    let mut output = format!("# Decisions\n\n{DECISIONS_MANAGED_COMMENT}\n\n## 현재 결정\n\n");
    if current.is_empty() {
        output.push_str("- 아직 기록된 확정 결정이 없습니다.\n");
    } else {
        for (date, title, decision, rationale, evidence) in current {
            output.push_str(&format!("### {date}: {title}\n\n- 결정: {decision}\n"));
            if !is_none_value(&rationale) {
                output.push_str(&format!("- 근거: {rationale}\n"));
            }
            output.push_str(&format!(
                "- 기록: {}\n\n",
                evidence
                    .iter()
                    .map(|filename| format!("`{filename}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    output.push_str("\n## 변경·폐기된 결정\n\n");
    if superseded.is_empty() {
        output.push_str("- 아직 기록된 변경·폐기 결정이 없습니다.\n");
    } else {
        for (date, title, decision, rationale, evidence) in superseded {
            output.push_str(&format!("### {date}: {title}\n\n- 이전 결정: {decision}\n"));
            if !is_none_value(&rationale) {
                output.push_str(&format!("- 변경 이유: {rationale}\n"));
            }
            output.push_str(&format!(
                "- 기록: {}\n\n",
                evidence
                    .iter()
                    .map(|filename| format!("`{filename}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if let Some((filename, _)) = fallback_evidence {
        output.push_str(&format!(
            "\n## 기존 결정 원문 보관\n\n- 전체 원문: `{filename}`\n"
        ));
    }
    Ok(output)
}

async fn initialize_local(room: &AiRoom) -> Result<(), ApiError> {
    let room_dir = PathBuf::from(&room.local_root).join(ROOM_DIR);
    fs::create_dir_all(room_dir.join("sessions")).await?;
    fs::create_dir_all(room_dir.join(LIBRARY_DIR)).await?;
    migrate_root_room_documents(&room_dir).await?;
    for (relative, content) in initial_files(room) {
        let path = room_dir.join(relative);
        if path
            .file_name()
            .is_some_and(|name| name == "room.json" || name == "ROOM.md")
            || fs::metadata(&path).await.is_err()
        {
            fs::write(path, content).await?;
        }
    }
    let decisions_path = room_dir.join("decisions.md");
    if let Ok(decisions) = fs::read_to_string(&decisions_path).await
        && !decisions.contains(DECISIONS_MANAGED_COMMENT)
        && decisions.trim().len() > "# Decisions".len()
    {
        let backup_path = room_dir.join(LEGACY_DECISIONS_FILE);
        if fs::metadata(&backup_path).await.is_err() {
            fs::write(backup_path, decisions).await?;
        }
    }
    let tasks_path = room_dir.join("tasks.md");
    let tasks = fs::read_to_string(&tasks_path).await.unwrap_or_default();
    let migrated_tasks = ensure_task_update_section(&tasks);
    if migrated_tasks != tasks {
        fs::write(tasks_path, migrated_tasks).await?;
    }
    if fs::metadata(room_dir.join(SESSION_OVERRIDES_FILE))
        .await
        .is_err()
    {
        fs::write(room_dir.join(SESSION_OVERRIDES_FILE), "{}\n").await?;
    }
    let block = managed_agent_block(room);
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let path = PathBuf::from(&room.local_root).join(filename);
        let existing = fs::read_to_string(&path).await.unwrap_or_default();
        fs::write(path, upsert_managed_block(&existing, &block)).await?;
    }
    Ok(())
}

async fn migrate_root_room_documents(room_dir: &FsPath) -> Result<(), ApiError> {
    let mut entries = match fs::read_dir(room_dir).await {
        Ok(entries) => entries,
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_library_filename(&name) || is_reserved_room_filename(&name) {
            continue;
        }
        let target = room_dir.join(LIBRARY_DIR).join(&name);
        if fs::metadata(&target).await.is_ok() {
            continue;
        }
        let metadata = entry.metadata().await?;
        if metadata.is_file() && metadata.len() <= MAX_DOCUMENT_BYTES as u64 {
            fs::copy(entry.path(), target).await?;
        }
    }
    Ok(())
}

async fn prepare_remote(room: &AiRoom, alias: &str, root: &str) -> Result<(), ApiError> {
    let local = read_local_files(room).await;
    if !local.available {
        return Err(ApiError::BadRequest(
            "Install local room instructions before preparing the server".into(),
        ));
    }
    let existing_remote = read_remote_files(room).await;
    if existing_remote
        .files
        .keys()
        .any(|name| name.starts_with("sessions/"))
    {
        upgrade_remote_instructions(
            room,
            alias,
            root,
            existing_remote.files.get("ROOM.md").map(String::as_str),
        )
        .await?;
        return Ok(());
    }

    let mut files = initial_files(room)
        .into_iter()
        .filter(|(relative, _)| relative == "room.json" || relative == "ROOM.md")
        .collect::<Vec<_>>();
    for relative in ["context.md", "decisions.md", "tasks.md"] {
        let content = local.files.get(relative).cloned().ok_or_else(|| {
            ApiError::BadRequest(format!("Local room document is missing: {relative}"))
        })?;
        files.push((relative.to_string(), content));
    }
    let mut library_baseline = BTreeMap::new();
    for (relative, content) in local
        .files
        .iter()
        .filter(|(relative, _)| relative.starts_with("library/"))
    {
        files.push((relative.clone(), content.clone()));
        library_baseline.insert(relative.clone(), content_hash(content));
    }
    files.push((
        LIBRARY_BASELINE_FILE.into(),
        serde_json::to_string_pretty(&library_baseline).unwrap(),
    ));
    let block = managed_agent_block(room);
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let existing = read_remote_root_file(alias, root, filename)
            .await
            .unwrap_or_default();
        files.push((filename.into(), upsert_managed_block(&existing, &block)));
    }
    write_remote_files(alias, root, &files).await
}

async fn clean_remote_room(
    alias: &str,
    root: &str,
    remote_files: &BTreeMap<String, String>,
) -> Result<Vec<String>, ApiError> {
    let mut script = format!(
        "root={}; room=\"$root/{}\"; for filename in AGENTS.md CLAUDE.md; do file=\"$root/$filename\"; [ -f \"$file\" ] || continue; tmp=\"$file.task-ai-room.$$.tmp\"; awk -v start='{}' -v end='{}' '$0 == start {{ skipping = 1; next }} $0 == end {{ skipping = 0; next }} !skipping {{ print }}' \"$file\" > \"$tmp\" && mv \"$tmp\" \"$file\" || exit 1; [ -s \"$file\" ] || rm -f \"$file\"; done; for name in room.json ROOM.md context.md decisions.md tasks.md {}; do rm -f \"$room/$name\" || exit 1; done; rm -f \"$room\"/sessions/*.md \"$room\"/library/*.md || exit 1;",
        posix_quote(root),
        ROOM_DIR,
        START_MARKER,
        END_MARKER,
        LIBRARY_BASELINE_FILE,
    );
    for filename in remote_files
        .keys()
        .filter_map(|name| name.strip_prefix("root-documents/"))
    {
        script.push_str(&format!(
            " rm -f \"$room/{}\" || exit 1;",
            filename.replace('"', "")
        ));
    }
    script.push_str(
        " rmdir \"$room/sessions\" 2>/dev/null || true; rmdir \"$room/library\" 2>/dev/null || true; rmdir \"$room\" 2>/dev/null || true;",
    );
    let output = run_remote(alias, &script).await?;
    if !output.status.success() {
        return Err(ApiError::BadRequest(format!(
            "Unable to remove temporary room records on {alias}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(vec![
        "AGENTS.md managed block".into(),
        "CLAUDE.md managed block".into(),
        format!("{ROOM_DIR}/"),
    ])
}

async fn delete_remote_library_file(
    alias: &str,
    root: &str,
    filename: &str,
) -> Result<(), ApiError> {
    let root_document = if is_reserved_room_filename(filename) {
        String::new()
    } else {
        " \"$room/$name\"".into()
    };
    let script = format!(
        "root={}; room=\"$root/{}\"; name={}; rm -f \"$room/library/$name\"{}",
        posix_quote(root),
        ROOM_DIR,
        posix_quote(filename),
        root_document,
    );
    let output = run_remote(alias, &script).await?;
    if !output.status.success() {
        return Err(ApiError::BadRequest(format!(
            "Unable to delete room document on {alias}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

async fn write_local_file(room: &AiRoom, relative: &str, content: &str) -> Result<(), ApiError> {
    let path = safe_room_path(room, relative)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, content).await?;
    Ok(())
}

fn safe_room_path(room: &AiRoom, relative: &str) -> Result<PathBuf, ApiError> {
    if relative.is_empty()
        || relative.contains(['\\', '\0', '\r', '\n'])
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ApiError::BadRequest("Invalid room record path".into()));
    }
    Ok(PathBuf::from(&room.local_root)
        .join(ROOM_DIR)
        .join(relative))
}

async fn write_remote_files(
    alias: &str,
    root: &str,
    files: &[(String, String)],
) -> Result<(), ApiError> {
    let quoted_root = posix_quote(root);
    let mut script = format!(
        "root={quoted_root}; mkdir -p \"$root/{ROOM_DIR}/sessions\" \"$root/{ROOM_DIR}/library\" || exit 1;"
    );
    for (relative, content) in files {
        if content.len() > MAX_DOCUMENT_BYTES {
            return Err(ApiError::PayloadTooLarge);
        }
        let target = if relative == "AGENTS.md" || relative == "CLAUDE.md" {
            format!("$root/{relative}")
        } else {
            format!("$root/{ROOM_DIR}/{relative}")
        };
        let parent = FsPath::new(relative)
            .parent()
            .and_then(FsPath::to_str)
            .unwrap_or("");
        if !parent.is_empty() {
            script.push_str(&format!(
                " mkdir -p \"$root/{ROOM_DIR}/{parent}\" || exit 1;"
            ));
        }
        script.push_str(&format!(
            " printf '%s' {} | base64 -d > \"{}\" || exit 1;",
            posix_quote(&BASE64.encode(content)),
            target
        ));
    }
    let output = run_remote(alias, &script).await?;
    if !output.status.success() {
        return Err(ApiError::BadRequest(format!(
            "Unable to write room files on {alias}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

async fn read_remote_root_file(
    alias: &str,
    root: &str,
    relative: &str,
) -> Result<String, ApiError> {
    let script = format!(
        "root={}; file=\"$root/{}\"; if [ -f \"$file\" ]; then base64 \"$file\" | tr -d '\\n'; fi",
        posix_quote(root),
        relative
    );
    let output = run_remote(alias, &script).await?;
    if !output.status.success() {
        return Err(ApiError::BadRequest(
            "Unable to read remote room file".into(),
        ));
    }
    let encoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if encoded.is_empty() {
        return Ok(String::new());
    }
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| ApiError::BadRequest("Invalid remote room file encoding".into()))?;
    String::from_utf8(bytes)
        .map_err(|_| ApiError::BadRequest("Remote room file is not UTF-8".into()))
}

async fn run_remote(alias: &str, script: &str) -> Result<std::process::Output, ApiError> {
    require_registered_alias(alias).await?;
    let args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=8".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-o".into(),
        CLEAR_SEND_ENV.into(),
        alias.into(),
        remote_shell_command(script),
    ];
    run_ssh(&args, Duration::from_secs(20)).await
}

async fn read_local_files(room: &AiRoom) -> EndpointFiles {
    let room_dir = PathBuf::from(&room.local_root).join(ROOM_DIR);
    let mut result = EndpointFiles::default();
    if fs::metadata(&room_dir).await.is_err() {
        result.error = Some("Room instructions are not installed locally".into());
        return result;
    }
    result.available = true;
    for filename in ["ROOM.md", "context.md", "decisions.md", "tasks.md"] {
        if let Ok(content) = fs::read_to_string(room_dir.join(filename)).await {
            result.files.insert(filename.into(), content);
        }
    }
    if let Ok(mut entries) = fs::read_dir(room_dir.join("sessions")).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".md")
                && let Ok(content) = fs::read_to_string(entry.path()).await
                && content.len() <= MAX_DOCUMENT_BYTES
            {
                result.files.insert(format!("sessions/{name}"), content);
            }
        }
    }
    if let Ok(mut entries) = fs::read_dir(room_dir.join(LIBRARY_DIR)).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_library_filename(&name)
                && let Ok(content) = fs::read_to_string(entry.path()).await
                && content.len() <= MAX_DOCUMENT_BYTES
            {
                result.files.insert(format!("library/{name}"), content);
            }
        }
    }
    result
}

async fn read_remote_files(room: &AiRoom) -> EndpointFiles {
    let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) else {
        return EndpointFiles::default();
    };
    let script = format!(
        r#"root={root}; room="$root/{room_dir}"; [ -d "$room" ] || exit 2;
for name in ROOM.md context.md decisions.md tasks.md {baseline}; do
  file="$room/$name"; [ -f "$file" ] || continue
  printf '%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done
for file in "$room"/*.md; do
  [ -f "$file" ] || continue
  name=$(basename "$file")
  case "$name" in ROOM.md|context.md|decisions.md|tasks.md) continue ;; esac
  printf 'root-documents/%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done
for file in "$room"/library/*.md; do
  [ -f "$file" ] || continue
  name=$(basename "$file")
  printf 'library/%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done
for file in "$room"/sessions/*.md; do
  [ -f "$file" ] || continue
  name=$(basename "$file")
  printf 'sessions/%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done"#,
        root = posix_quote(root),
        room_dir = ROOM_DIR,
        baseline = LIBRARY_BASELINE_FILE,
    );
    let output = match run_remote(alias, &script).await {
        Ok(output) => output,
        Err(error) => {
            return EndpointFiles {
                error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    if !output.status.success() {
        return EndpointFiles {
            error: Some(if output.status.code() == Some(2) {
                "Server is not prepared for a temporary AI session".into()
            } else {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            }),
            ..Default::default()
        };
    }
    let mut result = EndpointFiles {
        available: true,
        ..Default::default()
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((filename, encoded)) = line.split_once('\t') else {
            continue;
        };
        if let Ok(bytes) = BASE64.decode(encoded)
            && let Ok(content) = String::from_utf8(bytes)
            && content.len() <= MAX_DOCUMENT_BYTES
            && (filename == LIBRARY_BASELINE_FILE
                || (!filename.starts_with("library/") && !filename.starts_with("root-documents/"))
                || filename
                    .strip_prefix("library/")
                    .or_else(|| filename.strip_prefix("root-documents/"))
                    .is_some_and(valid_library_filename))
        {
            result.files.insert(filename.to_string(), content);
        }
    }
    result
}

fn remote_library_documents(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut documents = BTreeMap::new();
    for (filename, content) in files.iter().filter_map(|(name, content)| {
        name.strip_prefix("root-documents/")
            .map(|filename| (filename, content))
    }) {
        documents.insert(format!("library/{filename}"), content.clone());
    }
    for (filename, content) in files
        .iter()
        .filter(|(name, _)| name.starts_with("library/"))
    {
        documents.insert(filename.clone(), content.clone());
    }
    documents
}

async fn build_snapshot(room: AiRoom) -> AiRoomSnapshot {
    let local = read_local_files(&room).await;
    let remote = read_remote_files(&room).await;
    let mut sessions = Vec::new();
    let mut conflicts = Vec::new();
    let mut names = local
        .files
        .keys()
        .chain(remote.files.keys())
        .filter(|name| name.starts_with("sessions/"))
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names.reverse();
    for filename in names {
        match (local.files.get(&filename), remote.files.get(&filename)) {
            (Some(left), Some(right)) if left == right => sessions.push(AiRoomRecord {
                filename,
                content: left.clone(),
                source: "both".into(),
            }),
            (Some(left), Some(right)) if right.starts_with(left) => sessions.push(AiRoomRecord {
                filename,
                content: right.clone(),
                source: "remote".into(),
            }),
            (Some(left), Some(right)) if left.starts_with(right) => sessions.push(AiRoomRecord {
                filename,
                content: left.clone(),
                source: "local".into(),
            }),
            (Some(left), Some(_)) => {
                conflicts.push(filename.clone());
                sessions.push(AiRoomRecord {
                    filename,
                    content: left.clone(),
                    source: "conflict".into(),
                });
            }
            (Some(content), None) => sessions.push(AiRoomRecord {
                filename,
                content: content.clone(),
                source: "local".into(),
            }),
            (None, Some(content)) => sessions.push(AiRoomRecord {
                filename,
                content: content.clone(),
                source: "remote".into(),
            }),
            _ => {}
        }
    }
    let baseline = library_baseline(&remote.files);
    let remote_library = remote_library_documents(&remote.files);
    let mut library = Vec::new();
    for filename in ["ROOM.md", "context.md"] {
        match (local.files.get(filename), remote.files.get(filename)) {
            (Some(left), Some(right)) if left == right => library.push(AiRoomRecord {
                filename: filename.into(),
                content: left.clone(),
                source: "both".into(),
            }),
            (Some(left), Some(_)) => {
                conflicts.push(filename.into());
                library.push(AiRoomRecord {
                    filename: filename.into(),
                    content: left.clone(),
                    source: "conflict".into(),
                });
            }
            (Some(content), None) => library.push(AiRoomRecord {
                filename: filename.into(),
                content: content.clone(),
                source: "local".into(),
            }),
            (None, Some(content)) => library.push(AiRoomRecord {
                filename: filename.into(),
                content: content.clone(),
                source: "remote".into(),
            }),
            _ => {}
        }
    }
    for filename in ["decisions.md", "tasks.md"] {
        match (local.files.get(filename), remote.files.get(filename)) {
            (Some(content), _) => library.push(AiRoomRecord {
                filename: filename.into(),
                content: content.clone(),
                source: "local".into(),
            }),
            (None, Some(content)) => library.push(AiRoomRecord {
                filename: filename.into(),
                content: content.clone(),
                source: "remote".into(),
            }),
            _ => {}
        }
    }
    let mut library_names = local
        .files
        .keys()
        .chain(remote_library.keys())
        .filter(|name| name.starts_with("library/"))
        .cloned()
        .collect::<Vec<_>>();
    library_names.sort();
    library_names.dedup();
    for filename in library_names {
        match (local.files.get(&filename), remote_library.get(&filename)) {
            (Some(left), Some(right)) => {
                let action =
                    library_merge_action(left, right, baseline.get(&filename).map(String::as_str));
                let (content, source) = match action {
                    LibraryMergeAction::Same => (left.clone(), "both"),
                    LibraryMergeAction::AcceptRemote => (right.clone(), "remote"),
                    LibraryMergeAction::KeepLocal => (left.clone(), "local"),
                    LibraryMergeAction::Conflict => {
                        conflicts.push(filename.clone());
                        (left.clone(), "conflict")
                    }
                };
                library.push(AiRoomRecord {
                    filename,
                    content,
                    source: source.into(),
                });
            }
            (Some(content), None) => library.push(AiRoomRecord {
                filename,
                content: content.clone(),
                source: "local".into(),
            }),
            (None, Some(content)) => library.push(AiRoomRecord {
                filename,
                content: content.clone(),
                source: "remote".into(),
            }),
            _ => {}
        }
    }
    let value = |name: &str| {
        local
            .files
            .get(name)
            .or_else(|| remote.files.get(name))
            .cloned()
            .unwrap_or_default()
    };
    AiRoomSnapshot {
        instruction: value("ROOM.md"),
        context: value("context.md"),
        decisions: value("decisions.md"),
        tasks: value("tasks.md"),
        sessions,
        session_overrides: read_session_overrides(&PathBuf::from(&room.local_root).join(ROOM_DIR))
            .await,
        library,
        conflicts,
        local: AiRoomEndpointState {
            configured: true,
            available: local.available,
            instruction_installed: local.files.contains_key("ROOM.md"),
            error: local.error,
        },
        remote: AiRoomEndpointState {
            configured: room.ssh_alias.is_some(),
            available: remote.available,
            instruction_installed: remote.files.contains_key("ROOM.md"),
            error: remote.error,
        },
        room,
    }
}

fn document_path(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "context" => Ok("context.md"),
        "decisions" => Ok("decisions.md"),
        "tasks" => Ok("tasks.md"),
        _ => Err(ApiError::BadRequest("Unknown room document".into())),
    }
}

fn valid_library_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 120
        && filename.ends_with(".md")
        && !filename.starts_with('.')
        && !filename.contains(['/', '\\', '\0', '\r', '\n', '\t'])
        && filename
            .chars()
            .all(|value| value.is_alphanumeric() || matches!(value, '-' | '_' | ' ' | '.'))
}

fn is_reserved_room_filename(filename: &str) -> bool {
    matches!(
        filename,
        "ROOM.md" | "context.md" | "decisions.md" | "tasks.md"
    )
}

fn library_path(filename: &str) -> Result<String, ApiError> {
    if !valid_library_filename(filename) {
        return Err(ApiError::BadRequest(
            "Library filename must be a safe Markdown filename ending in .md".into(),
        ));
    }
    Ok(format!("{LIBRARY_DIR}/{filename}"))
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/ai-rooms", get(list_rooms).post(create_room))
        .route(
            "/ai-rooms/{room_id}",
            get(get_room_snapshot).delete(delete_room),
        )
        .route("/ai-rooms/{room_id}/initialize", post(initialize_room))
        .route(
            "/ai-rooms/{room_id}/prepare-remote",
            post(prepare_remote_room),
        )
        .route("/ai-rooms/{room_id}/sync", post(sync_room))
        .route(
            "/ai-rooms/{room_id}/import-remote-documents",
            post(import_remote_documents),
        )
        .route(
            "/ai-rooms/{room_id}/session-status",
            put(update_session_status),
        )
        .route("/ai-rooms/{room_id}/documents/{kind}", put(update_document))
        .route(
            "/ai-rooms/{room_id}/library/{filename}",
            put(update_library_file).delete(delete_library_file),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_agent_instructions() {
        let block = format!("{START_MARKER}\nnew\n{END_MARKER}");
        assert_eq!(
            upsert_managed_block("existing", &block),
            format!("existing\n\n{block}\n")
        );
        assert_eq!(
            upsert_managed_block(
                &format!("before\n{START_MARKER}\nold\n{END_MARKER}\nafter"),
                &block
            ),
            format!("before\n{block}\nafter")
        );
    }

    #[test]
    fn rejects_room_path_traversal() {
        let room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: None,
            local_root: "C:/work".into(),
            ssh_alias: None,
            remote_root: None,
            instruction_version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(safe_room_path(&room, "sessions/a.md").is_ok());
        assert!(safe_room_path(&room, "../secret").is_err());
        assert!(safe_room_path(&room, "sessions\\a.md").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn removes_windows_extended_path_prefix_for_room_display() {
        assert_eq!(
            normalized_local_path(FsPath::new(r"\\?\C:\work\project")),
            r"C:\work\project"
        );
        assert_eq!(
            normalized_local_path(FsPath::new(r"\\?\UNC\server\share\project")),
            r"\\server\share\project"
        );
    }

    #[test]
    fn waits_until_every_remote_session_is_complete() {
        let mut files = BTreeMap::new();
        files.insert("tasks.md".into(), "# Tasks".into());
        assert!(!all_remote_sessions_complete(&files));

        files.insert(
            "sessions/one.md".into(),
            format!("finished\n{SESSION_COMPLETE_MARKER}\n"),
        );
        assert!(all_remote_sessions_complete(&files));

        files.insert(
            "sessions/two.md".into(),
            format!("{SESSION_COMPLETE_MARKER}\nstill working"),
        );
        assert!(!all_remote_sessions_complete(&files));
    }

    #[test]
    fn summarizes_stable_work_without_waiting_for_chat_completion() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        assert!(session_is_ready_for_summary(
            "work checkpoint",
            Some(now - SESSION_STABLE_AFTER),
            now
        ));
        assert!(!session_is_ready_for_summary(
            "still being edited",
            Some(now - Duration::from_secs(30)),
            now
        ));
        assert!(session_is_ready_for_summary(
            &format!("finished\n{SESSION_COMPLETE_MARKER}\n"),
            Some(now),
            now
        ));
    }

    #[test]
    fn adds_task_update_section_without_rewriting_existing_tasks() {
        let original = "# Tasks\n\n- [ ] User task\n";
        let migrated = ensure_task_update_section(original);
        assert!(migrated.starts_with(original.trim_end()));
        assert!(migrated.contains("## 진행 중"));
        assert!(migrated.contains("## 완료"));
        assert_eq!(ensure_task_update_section(&migrated), migrated);
    }

    #[test]
    fn session_collection_hash_is_stable_and_content_sensitive() {
        let sessions = vec![
            ("sessions/one.md".into(), "first".into()),
            ("sessions/two.md".into(), "second".into()),
        ];
        assert_eq!(
            session_collection_hash(&sessions),
            session_collection_hash(&sessions)
        );
        let changed = vec![
            ("sessions/one.md".into(), "first".into()),
            ("sessions/two.md".into(), "updated".into()),
        ];
        assert_ne!(
            session_collection_hash(&sessions),
            session_collection_hash(&changed)
        );
    }

    #[test]
    fn renders_a_consolidated_korean_task_dashboard() {
        let rendered = render_task_dashboard(TaskDashboard {
            items: vec![
                TaskDashboardItem {
                    status: "active".into(),
                    title: "벤치마크 실행".into(),
                    next: "가중치 찾기".into(),
                    blocked: "모델 경로 미확인".into(),
                },
                TaskDashboardItem {
                    status: "active".into(),
                    title: "벤치마크 실행".into(),
                    next: "중복 항목".into(),
                    blocked: "none".into(),
                },
                TaskDashboardItem {
                    status: "completed".into(),
                    title: "결과 폴더 정리".into(),
                    next: "none".into(),
                    blocked: "none".into(),
                },
                TaskDashboardItem {
                    status: "active".into(),
                    title: "결과 폴더 정리".into(),
                    next: "오래된 상태".into(),
                    blocked: "none".into(),
                },
                TaskDashboardItem {
                    status: "completed".into(),
                    title: "표 | 통합\n완료".into(),
                    next: "none".into(),
                    blocked: "none".into(),
                },
            ],
        })
        .unwrap();

        assert!(rendered.contains("## 진행 중"));
        assert!(rendered.contains("## 완료"));
        assert_eq!(rendered.matches("- [ ] 벤치마크 실행").count(), 1);
        assert!(!rendered.contains("- [ ] 결과 폴더 정리"));
        assert!(rendered.contains("- [x] 결과 폴더 정리"));
        assert!(rendered.contains("- [x] 표 / 통합 완료"));
        assert!(rendered.contains("  - 다음: 가중치 찾기"));
        assert!(rendered.contains("  - 차단: 모델 경로 미확인"));
    }

    #[test]
    fn rejects_an_empty_task_dashboard() {
        assert!(
            render_task_dashboard(TaskDashboard { items: Vec::new() })
                .unwrap_err()
                .contains("empty")
        );
    }

    #[test]
    fn suppresses_empty_next_steps_and_blockers() {
        let rendered = render_task_dashboard(TaskDashboard {
            items: vec![TaskDashboardItem {
                status: "active".into(),
                title: "구현 검증".into(),
                next: "run tests".into(),
                blocked: "해당 없음".into(),
            }],
        })
        .unwrap();
        assert!(rendered.contains("  - 다음: run tests"));
        assert!(!rendered.contains("  - 차단:"));
    }

    #[test]
    fn restores_latest_explicit_follow_up_when_model_omits_active_work() {
        let dashboard = TaskDashboard {
            items: vec![TaskDashboardItem {
                status: "completed".into(),
                title: "exp33 완료".into(),
                next: "none".into(),
                blocked: "none".into(),
            }],
        };
        let sessions = vec![(
            "sessions/20260727-091700-claude-exp33.md".into(),
            "# Session: exp33\n\n- 후속=pair-identity 분기(exp34+).\n".into(),
        )];
        let rendered = render_task_dashboard(ensure_latest_next_action(
            dashboard,
            &sessions,
            &BTreeMap::new(),
        ))
        .unwrap();
        assert!(rendered.contains("- [ ] pair-identity 분기(exp34+)."));
    }

    #[test]
    fn removes_stopped_sessions_from_model_output() {
        let dashboard = TaskDashboard {
            items: vec![
                TaskDashboardItem {
                    status: "completed".into(),
                    title: "FMPose3D service-compatible experiment (stopped)".into(),
                    next: "none".into(),
                    blocked: "none".into(),
                },
                TaskDashboardItem {
                    status: "completed".into(),
                    title: "Keep this result".into(),
                    next: "none".into(),
                    blocked: "none".into(),
                },
            ],
        };
        let sessions = vec![(
            "sessions/stopped.md".into(),
            "# Session: FMPose3D service-compatible experiment\n\n## Status\n\n- In progress.\n"
                .into(),
        )];
        let overrides = BTreeMap::from([("sessions/stopped.md".into(), "stopped".into())]);

        let rendered =
            render_task_dashboard(remove_stopped_sessions(dashboard, &sessions, &overrides))
                .unwrap();
        assert!(!rendered.contains("FMPose3D"));
        assert!(rendered.contains("- [x] Keep this result"));
    }

    #[test]
    fn renders_only_evidence_backed_decisions() {
        let sessions = vec![(
            "sessions/20260727-090000-claude-test.md".into(),
            "decision evidence".into(),
        )];
        let rendered = render_decision_dashboard(
            DecisionDashboard {
                items: vec![
                    DecisionDashboardItem {
                        date: "2024-10-31".into(),
                        title: "동기화 방식".into(),
                        decision: "진행 중 체크포인트를 비파괴 복사한다".into(),
                        rationale: "강제 종료에도 로컬 기록을 보존한다".into(),
                        status: "current".into(),
                        evidence: vec!["20260727-090000-claude-test.md".into()],
                    },
                    DecisionDashboardItem {
                        date: "2026-07-27".into(),
                        title: "근거 없는 결정".into(),
                        decision: "포함하면 안 된다".into(),
                        rationale: "none".into(),
                        status: "current".into(),
                        evidence: vec!["sessions/missing.md".into()],
                    },
                ],
            },
            &sessions,
            None,
        )
        .unwrap();
        assert!(rendered.contains("### 2026-07-27: 동기화 방식"));
        assert!(rendered.contains("진행 중 체크포인트를 비파괴 복사한다"));
        assert!(!rendered.contains("근거 없는 결정"));
        assert!(rendered.contains(DECISIONS_MANAGED_COMMENT));
    }

    #[test]
    fn preserves_explicit_decisions_from_session_checkpoints() {
        let sessions = vec![(
            "sessions/20260727-091700-claude-depth-ground-projection.md".into(),
            "# Session: 깊이 접지 실험\n\n**결정: 단안 3D 접지 접음.**\n\n판정: t=0.5 무조건 이동은 틀린 타깃 → 폐기.\n"
                .into(),
        )];
        let dashboard =
            augment_explicit_decisions(DecisionDashboard { items: Vec::new() }, &sessions);
        let rendered = render_decision_dashboard(dashboard, &sessions, None).unwrap();

        assert!(rendered.contains("### 2026-07-27: 깊이 접지 실험 — 명시적 결정"));
        assert!(rendered.contains("단안 3D 접지 접음."));
        assert!(rendered.contains("t=0.5 무조건 이동은 틀린 타깃 → 폐기."));
        assert!(rendered.contains("`sessions/20260727-091700-claude-depth-ground-projection.md`"));
    }

    #[test]
    fn bounds_utf8_prompt_without_breaking_characters() {
        let source = "가나다라마바사아자차카타파하";
        let bounded = bounded_text(source, 18);
        assert!(bounded.starts_with("가나다"));
        assert!(bounded.contains("...[truncated]..."));
        assert!(bounded.ends_with('하'));
    }

    #[test]
    fn validates_flat_markdown_library_filenames() {
        assert!(valid_library_filename("review-checklist.md"));
        assert!(valid_library_filename("배포 절차.md"));
        assert!(!valid_library_filename("../secret.md"));
        assert!(!valid_library_filename("nested/method.md"));
        assert!(!valid_library_filename(".hidden.md"));
        assert!(!valid_library_filename("binary.exe"));
    }

    #[test]
    fn maps_server_root_markdown_into_room_documents() {
        let mut files = BTreeMap::new();
        files.insert(
            "root-documents/deploy-notes.md".into(),
            "# Server notes".into(),
        );
        files.insert("library/review-method.md".into(), "# Review method".into());

        let documents = remote_library_documents(&files);

        assert_eq!(
            documents.get("library/deploy-notes.md"),
            Some(&"# Server notes".to_string())
        );
        assert_eq!(
            documents.get("library/review-method.md"),
            Some(&"# Review method".to_string())
        );
    }

    #[test]
    fn protects_managed_room_documents_from_library_deletion() {
        assert!(is_reserved_room_filename("context.md"));
        assert!(is_reserved_room_filename("ROOM.md"));
        assert!(!is_reserved_room_filename("deploy-notes.md"));
    }

    #[test]
    fn three_way_merges_library_documents_safely() {
        let base = "# Method\n\nOriginal";
        let local = "# Method\n\nLocal edit";
        let remote = "# Method\n\nRemote edit";
        let baseline = content_hash(base);

        assert_eq!(
            library_merge_action(base, remote, Some(&baseline)),
            LibraryMergeAction::AcceptRemote
        );
        assert_eq!(
            library_merge_action(local, base, Some(&baseline)),
            LibraryMergeAction::KeepLocal
        );
        assert_eq!(
            library_merge_action(local, remote, Some(&baseline)),
            LibraryMergeAction::Conflict
        );
        assert_eq!(
            library_merge_action(remote, remote, Some(&baseline)),
            LibraryMergeAction::Same
        );
        assert_eq!(
            library_merge_action(local, remote, None),
            LibraryMergeAction::Conflict
        );
    }

    #[tokio::test]
    async fn discovers_ai_created_library_documents() {
        let root = tempfile::tempdir().unwrap();
        let room_dir = root.path().join(ROOM_DIR);
        fs::create_dir_all(room_dir.join(LIBRARY_DIR))
            .await
            .unwrap();
        fs::write(
            room_dir.join(LIBRARY_DIR).join("review-method.md"),
            "# Review method",
        )
        .await
        .unwrap();
        let room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: None,
            local_root: normalized_local_path(root.path()),
            ssh_alias: None,
            remote_root: None,
            instruction_version: INSTRUCTION_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let files = read_local_files(&room).await;

        assert!(files.available);
        assert_eq!(
            files.files.get("library/review-method.md"),
            Some(&"# Review method".to_string())
        );
    }

    #[tokio::test]
    async fn fast_forwards_active_remote_session_without_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let room_dir = root.path().join(ROOM_DIR);
        fs::create_dir_all(room_dir.join("sessions")).await.unwrap();
        fs::write(room_dir.join("sessions/active.md"), "checkpoint one")
            .await
            .unwrap();
        let room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: None,
            local_root: normalized_local_path(root.path()),
            ssh_alias: None,
            remote_root: None,
            instruction_version: INSTRUCTION_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut local = read_local_files(&room).await;
        let remote = EndpointFiles {
            available: true,
            files: BTreeMap::from([(
                "sessions/active.md".into(),
                "checkpoint one\ncheckpoint two".into(),
            )]),
            ..Default::default()
        };

        let (copied, conflicts) = sync_remote_checkpoints(&room, &mut local, &remote)
            .await
            .unwrap();

        assert_eq!(copied, vec!["sessions/active.md"]);
        assert!(conflicts.is_empty());
        assert_eq!(
            fs::read_to_string(room_dir.join("sessions/active.md"))
                .await
                .unwrap(),
            "checkpoint one\ncheckpoint two"
        );
    }
}
