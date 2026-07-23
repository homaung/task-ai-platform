use std::{
    collections::BTreeMap,
    env,
    path::{Path as FsPath, PathBuf},
    time::Duration,
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
const INSTRUCTION_VERSION: i64 = 3;
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;
const CLEAR_SEND_ENV: &str = "SendEnv=-*";
const START_MARKER: &str = "<!-- task-ai-room:start -->";
const END_MARKER: &str = "<!-- task-ai-room:end -->";
const SESSION_COMPLETE_MARKER: &str = "<!-- task-ai-room:complete -->";
const AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(15);
const TASK_SUMMARY_INTERVAL: Duration = Duration::from_secs(45);
const TASK_SUMMARY_STATE_FILE: &str = "task-summary-state.json";
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_TASK_SUMMARY_MODEL: &str = "qwen3.5:4b";
const MAX_SESSION_PROMPT_BYTES: usize = 96 * 1024;
const MAX_TASKS_PROMPT_BYTES: usize = 32 * 1024;
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
    pub conflicts: Vec<String>,
    pub local: AiRoomEndpointState,
    pub remote: AiRoomEndpointState,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateAiRoomDocumentRequest {
    pub content: String,
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
    processed_sessions: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SessionTaskSummary {
    completed: bool,
    summary: String,
    next: String,
    blocked: String,
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
    let relative = document_path(&kind)?;
    let room = find_room(&deployment, room_id).await?;
    write_local_file(&room, relative, &payload.content).await?;
    AiRoom::touch(&deployment.db().pool, room.id).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
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
    let local = read_local_files(&room).await;
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

        for (filename, content) in remote
            .files
            .iter()
            .filter(|(name, _)| name.starts_with("sessions/"))
        {
            match local.files.get(filename) {
                None => {
                    write_local_file(&room, filename, content).await?;
                    copied_to_local.push(filename.clone());
                }
                Some(local_content) if local_content != content => conflicts.push(filename.clone()),
                _ => {}
            }
        }

        for document in ["context.md", "decisions.md"] {
            if let (Some(left), Some(right)) =
                (local.files.get(document), remote.files.get(document))
                && left != right
            {
                conflicts.push(document.to_string());
            }
        }

        if let (Some(local_tasks), Some(remote_tasks)) =
            (local.files.get("tasks.md"), remote.files.get("tasks.md"))
            && local_tasks != remote_tasks
        {
            if is_append_only_update(local_tasks, remote_tasks) {
                write_local_file(&room, "tasks.md", remote_tasks).await?;
                copied_to_local.push("tasks.md".into());
            } else {
                conflicts.push("tasks.md".into());
            }
        }

        if conflicts.is_empty() {
            removed_from_remote = clean_remote_room(alias, root).await?;
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
                let sessions_are_complete =
                    remote.available && all_remote_sessions_complete(&remote.files);
                if !sessions_are_complete {
                    continue;
                }

                let _guard = SYNC_LOCK.lock().await;
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
    format!(
        "# AI Room: {name}\n\nRoom ID: `{id}`\nInstruction version: {version}\n\n## Required session workflow\n\n1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, and the newest files in `.ai-room/sessions/`.\n2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.\n3. Record the goal, assumptions, important commands, files changed, verification results, decisions, blockers, and concrete next steps. Update it during the session, not only at the end.\n4. Do not edit the `## AI session updates` section in `tasks.md`. The Task AI Platform reads each completed session locally and appends one validated status line automatically. Make the result, completion state, next action, and blockers explicit in the session file so the local summarizer can report them accurately.\n5. Update `context.md` only for durable project facts. Append architectural decisions to `decisions.md`; do not rewrite history.\n6. Never store secrets, tokens, private keys, or raw credentials in room files.\n7. Before ending, make the session file sufficient for another Claude or Codex session to continue without relying on chat history. After all other writes are finished, add `{complete_marker}` as the final line of the session file. The app uses this exact marker to know the session is safe to synchronize, summarize locally, and remove from the server.\n\n## Server privacy\n\nWhen this room is prepared on its SSH server, all `.ai-room` files there are temporary. The Task AI Platform automatically copies completed session files to the local root, then removes the server-side room files after a conflict-free sync. The local task summarizer uses only the local Ollama service; session contents are not sent to a cloud model. Do not assume earlier session files remain on the server.\n\n## Room endpoints\n\n- Local root: `{local}`\n- Remote root: `{remote}`\n\nThe Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.\n",
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
            format!("# Context\n\nDurable project context for {}.\n", room.name),
        ),
        (
            "decisions.md".into(),
            "# Decisions\n\nAppend dated architectural and product decisions here.\n".into(),
        ),
        (
            "tasks.md".into(),
            "# Tasks\n\n## Planned work\n\n- [ ] Describe the next concrete task.\n\n## AI session updates\n\n<!-- The local task summarizer appends one validated line per completed session. -->\n"
                .into(),
        ),
    ]
}

fn is_append_only_update(local: &str, remote: &str) -> bool {
    remote.len() > local.len() && remote.starts_with(local)
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

fn ensure_task_update_section(content: &str) -> String {
    if content.contains("## AI session updates") {
        content.to_string()
    } else {
        format!(
            "{}\n\n## AI session updates\n\n<!-- The local task summarizer appends one validated line per completed session. -->\n",
            content.trim_end()
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
    let initial_tasks = fs::read_to_string(&tasks_path)
        .await
        .map_err(|error| format!("{} has no readable task list: {error}", room.name))?;
    let mut state = read_task_summary_state(&room_dir).await;
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
            && session_is_complete(&content)
        {
            sessions.push((format!("sessions/{name}"), content));
        }
    }
    sessions.sort_by(|left, right| left.0.cmp(&right.0));

    for (filename, content) in sessions {
        let hash = session_hash(&content);
        if state.processed_sessions.get(&filename) == Some(&hash) {
            continue;
        }
        if tasks_reference_session(&initial_tasks, &filename) {
            state.processed_sessions.insert(filename, hash);
            write_task_summary_state(&room_dir, &state).await?;
            continue;
        }

        let task_line = match extract_task_line(&content, &filename) {
            Some(line) => line,
            None => {
                let summary = request_local_task_summary(&content, &initial_tasks).await?;
                format_task_line(&filename, summary)?
            }
        };

        let mut current_tasks = fs::read_to_string(&tasks_path)
            .await
            .map_err(|error| format!("Unable to re-read {} tasks: {error}", room.name))?;
        if !tasks_reference_session(&current_tasks, &filename) {
            current_tasks = ensure_task_update_section(&current_tasks);
            if !current_tasks.ends_with('\n') {
                current_tasks.push('\n');
            }
            current_tasks.push_str(&task_line);
            current_tasks.push('\n');
            fs::write(&tasks_path, current_tasks)
                .await
                .map_err(|error| format!("Unable to update {} tasks: {error}", room.name))?;
        }

        state.processed_sessions.insert(filename.clone(), hash);
        write_task_summary_state(&room_dir, &state).await?;
        tracing::info!(room_id = %room.id, session = filename, "AI Room summarized a completed session locally");
        return Ok(());
    }

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

fn session_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn tasks_reference_session(tasks: &str, filename: &str) -> bool {
    tasks.lines().any(|line| {
        line.contains("Session:")
            && line
                .split('|')
                .any(|part| part.trim() == format!("Session: {filename}"))
    })
}

fn extract_task_line(content: &str, filename: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let candidate = line.trim();
        if candidate.len() > 1_024
            || !candidate.starts_with("- [")
            || !candidate.contains("| Done:")
            || !candidate.contains("| Next:")
            || !candidate.contains("| Blocked:")
        {
            return None;
        }
        if candidate.contains("| Session:") && !tasks_reference_session(candidate, filename) {
            return None;
        }
        let without_session = candidate.split("| Session:").next()?.trim_end();
        Some(format!("{without_session} | Session: {filename}"))
    })
}

async fn request_local_task_summary(
    session: &str,
    tasks: &str,
) -> Result<SessionTaskSummary, String> {
    let base_url = env::var("TASK_AI_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_URL.into());
    let model =
        env::var("TASK_AI_SUMMARY_MODEL").unwrap_or_else(|_| DEFAULT_TASK_SUMMARY_MODEL.into());
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "completed": { "type": "boolean" },
            "summary": { "type": "string" },
            "next": { "type": "string" },
            "blocked": { "type": "string" }
        },
        "required": ["completed", "summary", "next", "blocked"],
        "additionalProperties": false
    });
    let prompt = format!(
        "You maintain a concise project task dashboard from a completed AI coding session. Use only facts in the session. Return the requested JSON schema. Write in the session's main language. `completed` means the session goal was achieved, not merely that the session ended. Keep summary under 180 characters, next under 140, and blocked under 100. Use `none` when there is no next action or blocker. Do not include markdown or pipe characters.\n\nCURRENT TASKS:\n{}\n\nCOMPLETED SESSION:\n{}",
        bounded_text(tasks, MAX_TASKS_PROMPT_BYTES),
        bounded_text(session, MAX_SESSION_PROMPT_BYTES)
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
        .map_err(|error| format!("Ollama returned invalid task summary JSON: {error}"))
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

fn format_task_line(filename: &str, summary: SessionTaskSummary) -> Result<String, String> {
    let done = clean_summary_field(&summary.summary, 180, "");
    if done.is_empty() {
        return Err("Local model returned an empty task summary".into());
    }
    let next = clean_summary_field(&summary.next, 140, "none");
    let blocked = clean_summary_field(&summary.blocked, 100, "none");
    let checkbox = if summary.completed { "x" } else { " " };
    let (timestamp, agent) = session_identity(filename);
    Ok(format!(
        "- [{checkbox}] {timestamp} | {agent} | Done: {done} | Next: {next} | Blocked: {blocked} | Session: {filename}"
    ))
}

fn session_identity(filename: &str) -> (String, String) {
    let stem = filename
        .strip_prefix("sessions/")
        .and_then(|value| value.strip_suffix(".md"))
        .unwrap_or(filename);
    let mut parts = stem.split('-');
    let date = parts.next().unwrap_or_default();
    let time = parts.next().unwrap_or_default();
    let agent = clean_summary_field(parts.next().unwrap_or("ai"), 32, "ai");
    let timestamp = if date.len() == 8
        && time.len() == 6
        && date
            .chars()
            .chain(time.chars())
            .all(|value| value.is_ascii_digit())
    {
        format!(
            "{}-{}-{} {}:{}",
            &date[0..4],
            &date[4..6],
            &date[6..8],
            &time[0..2],
            &time[2..4]
        )
    } else {
        "unknown-time".into()
    };
    (timestamp, agent)
}

async fn initialize_local(room: &AiRoom) -> Result<(), ApiError> {
    let room_dir = PathBuf::from(&room.local_root).join(ROOM_DIR);
    fs::create_dir_all(room_dir.join("sessions")).await?;
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
    let tasks_path = room_dir.join("tasks.md");
    let tasks = fs::read_to_string(&tasks_path).await.unwrap_or_default();
    let migrated_tasks = ensure_task_update_section(&tasks);
    if migrated_tasks != tasks {
        fs::write(tasks_path, migrated_tasks).await?;
    }
    let block = managed_agent_block(room);
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let path = PathBuf::from(&room.local_root).join(filename);
        let existing = fs::read_to_string(&path).await.unwrap_or_default();
        fs::write(path, upsert_managed_block(&existing, &block)).await?;
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
        return Err(ApiError::BadRequest(
            "Server session records already exist. Sync them before preparing again".into(),
        ));
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
    let block = managed_agent_block(room);
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let existing = read_remote_root_file(alias, root, filename)
            .await
            .unwrap_or_default();
        files.push((filename.into(), upsert_managed_block(&existing, &block)));
    }
    write_remote_files(alias, root, &files).await
}

async fn clean_remote_room(alias: &str, root: &str) -> Result<Vec<String>, ApiError> {
    let script = format!(
        "root={}; room=\"$root/{}\"; for filename in AGENTS.md CLAUDE.md; do file=\"$root/$filename\"; [ -f \"$file\" ] || continue; tmp=\"$file.task-ai-room.$$.tmp\"; awk -v start='{}' -v end='{}' '$0 == start {{ skipping = 1; next }} $0 == end {{ skipping = 0; next }} !skipping {{ print }}' \"$file\" > \"$tmp\" && mv \"$tmp\" \"$file\" || exit 1; [ -s \"$file\" ] || rm -f \"$file\"; done; for name in room.json ROOM.md context.md decisions.md tasks.md; do rm -f \"$room/$name\" || exit 1; done; rm -f \"$room\"/sessions/*.md || exit 1; rmdir \"$room/sessions\" 2>/dev/null || true; rmdir \"$room\" 2>/dev/null || true;",
        posix_quote(root),
        ROOM_DIR,
        START_MARKER,
        END_MARKER,
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
    let mut script =
        format!("root={quoted_root}; mkdir -p \"$root/{ROOM_DIR}/sessions\" || exit 1;");
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
    result
}

async fn read_remote_files(room: &AiRoom) -> EndpointFiles {
    let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) else {
        return EndpointFiles::default();
    };
    let script = format!(
        r#"root={root}; room="$root/{room_dir}"; [ -d "$room" ] || exit 2;
for name in ROOM.md context.md decisions.md tasks.md; do
  file="$room/$name"; [ -f "$file" ] || continue
  printf '%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done
for file in "$room"/sessions/*.md; do
  [ -f "$file" ] || continue
  name=$(basename "$file")
  printf 'sessions/%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done"#,
        root = posix_quote(root),
        room_dir = ROOM_DIR,
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
        {
            result.files.insert(filename.to_string(), content);
        }
    }
    result
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
        .route("/ai-rooms/{room_id}/documents/{kind}", put(update_document))
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
    fn accepts_only_append_only_task_updates() {
        let local = "# Tasks\n\n## AI session updates\n";
        assert!(is_append_only_update(
            local,
            &format!("{local}- [x] one completed session\n")
        ));
        assert!(!is_append_only_update(
            local,
            "# Tasks\n\nrewritten content\n"
        ));
        assert!(!is_append_only_update(local, local));
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
    fn adds_task_update_section_without_rewriting_existing_tasks() {
        let original = "# Tasks\n\n- [ ] User task\n";
        let migrated = ensure_task_update_section(original);
        assert!(migrated.starts_with(original.trim_end()));
        assert!(migrated.contains("## AI session updates"));
        assert_eq!(ensure_task_update_section(&migrated), migrated);
    }

    #[test]
    fn recognizes_only_the_exact_session_reference() {
        let tasks = "- [x] done | Session: sessions/one.md\n";
        assert!(tasks_reference_session(tasks, "sessions/one.md"));
        assert!(!tasks_reference_session(tasks, "sessions/two.md"));
    }

    #[test]
    fn reuses_a_valid_task_line_from_the_session() {
        let content = "notes\n- [x] 2026-07-23 10:20 | codex | Done: fixed sync | Next: none | Blocked: none\n";
        assert_eq!(
            extract_task_line(content, "sessions/20260723-102000-codex-a1.md").unwrap(),
            "- [x] 2026-07-23 10:20 | codex | Done: fixed sync | Next: none | Blocked: none | Session: sessions/20260723-102000-codex-a1.md"
        );
    }

    #[test]
    fn formats_and_sanitizes_model_task_summary() {
        let line = format_task_line(
            "sessions/20260723-102030-claude-a1.md",
            SessionTaskSummary {
                completed: false,
                summary: "implemented | verified\nlocally".into(),
                next: "run tests".into(),
                blocked: "".into(),
            },
        )
        .unwrap();
        assert_eq!(
            line,
            "- [ ] 2026-07-23 10:20 | claude | Done: implemented / verified locally | Next: run tests | Blocked: none | Session: sessions/20260723-102030-claude-a1.md"
        );
    }

    #[test]
    fn bounds_utf8_prompt_without_breaking_characters() {
        let source = "가나다라마바사아자차카타파하";
        let bounded = bounded_text(source, 18);
        assert!(bounded.starts_with("가나다"));
        assert!(bounded.contains("...[truncated]..."));
        assert!(bounded.ends_with('하'));
    }
}
