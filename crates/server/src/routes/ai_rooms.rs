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
use db::models::ai_room::{AiRoom, AiRoomLocalIdentity, AiRoomStorageProfile, CreateAiRoom};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex};
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    error::ApiError,
    routes::ssh_hosts::{
        posix_quote, remote_shell_command, require_registered_alias, run_ssh, run_ssh_with_stdin,
    },
};

const ROOM_DIR: &str = ".ai-room";
const ROOM_GITIGNORE_ENTRY: &str = "/.ai-room/";
const LIBRARY_DIR: &str = "library";
const OWNER_RULES_NAME: &str = "owner-working-rules.md";
const OWNER_RULES_FILE: &str = "library/owner-working-rules.md";
const ADVERSARIAL_REVIEW_NAME: &str = "adversarial-code-review-protocol.md";
const ADVERSARIAL_REVIEW_FILE: &str = "library/adversarial-code-review-protocol.md";
const LOCAL_HISTORY_DIR: &str = "local-history";
const LEGACY_DECISIONS_FILE: &str = "library/legacy-decisions.md";
const LIBRARY_BASELINE_FILE: &str = ".library-baseline.json";
const SESSION_OVERRIDES_FILE: &str = "session-overrides.json";
const DECISIONS_MANAGED_COMMENT: &str =
    "<!-- Task AI Platform가 안정된 AI 작업 기록에서 확정된 결정을 자동 정리합니다. -->";
const INSTRUCTION_VERSION: i64 = 16;
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;
const CLEAR_SEND_ENV: &str = "SendEnv=-*";
const START_MARKER: &str = "<!-- task-ai-room:start -->";
const END_MARKER: &str = "<!-- task-ai-room:end -->";
const SESSION_COMPLETE_MARKER: &str = "<!-- task-ai-room:complete -->";
const SESSION_INDEX_FILE: &str = "INDEX.md";
const AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(15);
const TASK_SUMMARY_INTERVAL: Duration = Duration::from_secs(45);
const SESSION_STABLE_AFTER: Duration = Duration::from_secs(120);
const CHECKPOINT_OVERDUE_AFTER: Duration = Duration::from_secs(10 * 60);
const ACTIVITY_RECENT_WINDOW: Duration = Duration::from_secs(30 * 60);
const REMOTE_ACTIVITY_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
const LOCAL_ACTIVITY_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const ACTIVITY_SCAN_MAX_ENTRIES: usize = 20_000;
const ACTIVITY_SCAN_MAX_DEPTH: usize = 5;
const ACTIVITY_EXCLUDED_DIRS: &[&str] = &[
    ".ai-room",
    ".git",
    ".claude",
    ".codex",
    ".cache",
    ".venv",
    ".next",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    "venv",
];
/// A room whose dashboard rebuild keeps failing must not retry every
/// `TASK_SUMMARY_INTERVAL`. The local model holds the GPU for the whole
/// generation, so an unconvergeable room would pin it permanently.
const SUMMARY_RETRY_BACKOFF_BASE: Duration = Duration::from_secs(5 * 60);
const SUMMARY_RETRY_BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);
const LOCAL_SUMMARY_ENV: &str = "TASK_AI_LOCAL_SUMMARY";
const TASK_SUMMARY_STATE_FILE: &str = "task-summary-state.json";
const TASK_DASHBOARD_VERSION: u8 = 7;
const DECISION_DASHBOARD_VERSION: u8 = 4;
const LOCAL_ROOT_NOT_EMPTY_MARKER: &str = "LOCAL_ROOT_NOT_EMPTY";
const SERVER_NOT_PREPARED_ERROR: &str = "Server is not prepared for an AI room yet";
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_TASK_SUMMARY_MODEL: &str = "qwen3.5:4b";
const MAX_SESSION_PROMPT_BYTES: usize = 96 * 1024;
const MAX_TASKS_PROMPT_BYTES: usize = 32 * 1024;
const MAX_DECISIONS_PROMPT_BYTES: usize = 48 * 1024;
static SYNC_LOCK: Mutex<()> = Mutex::const_new(());
static SESSION_OVERRIDE_LOCK: Mutex<()> = Mutex::const_new(());
static TASK_SUMMARY_LOCK: Mutex<()> = Mutex::const_new(());
static REMOTE_ACTIVITY: Mutex<BTreeMap<Uuid, (SystemTime, Option<SystemTime>)>> =
    Mutex::const_new(BTreeMap::new());
static LOCAL_ACTIVITY: Mutex<BTreeMap<Uuid, (SystemTime, Option<SystemTime>)>> =
    Mutex::const_new(BTreeMap::new());
/// Consecutive dashboard failures per room and the time its next attempt is
/// allowed. Cleared as soon as a room summarizes successfully.
static SUMMARY_BACKOFF: Mutex<BTreeMap<Uuid, (u32, SystemTime)>> =
    Mutex::const_new(BTreeMap::new());

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
pub struct AiRoomCheckpointHealth {
    pub status: String,
    pub active_sessions: usize,
    pub overdue_sessions: usize,
    pub latest_session: Option<String>,
    pub latest_checkpoint_age_seconds: Option<u32>,
    pub overdue_after_minutes: u32,
    pub unrecorded_activity: bool,
    pub local_activity_age_seconds: Option<u32>,
    pub remote_activity_age_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct AiRoomSnapshot {
    pub room: AiRoom,
    pub instruction: String,
    pub context: String,
    pub decisions: String,
    pub tasks: String,
    pub sessions: Vec<AiRoomRecord>,
    pub checkpoint_health: AiRoomCheckpointHealth,
    /// False while the local model dashboard is off, so the UI can say that
    /// `tasks.md` and `decisions.md` are no longer rebuilt automatically.
    pub local_summary_enabled: bool,
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

#[derive(Debug, Deserialize, TS)]
pub struct UpdateAiRoomProfileRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateAiRoomConnectionRequest {
    pub ssh_alias: Option<String>,
    pub remote_root: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, TS)]
pub struct SyncAiRoomResponse {
    pub copied_to_local: Vec<String>,
    pub copied_to_remote: Vec<String>,
    pub removed_from_remote: Vec<String>,
    pub conflicts: Vec<String>,
    pub snapshot: AiRoomSnapshot,
}

#[derive(Debug, Serialize, TS)]
pub struct AiRoomStorageStatus {
    pub identity: AiRoomLocalIdentity,
    pub profile: AiRoomStorageProfile,
    pub task_ai_cloud_available: bool,
    pub personal_hub_available: bool,
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

    let requested_root = PathBuf::from(payload.local_root.trim());
    if requested_root.as_os_str().is_empty() {
        return Err(ApiError::BadRequest("Local root is required".into()));
    }
    let created_root =
        prepare_local_root(&requested_root, payload.allow_existing_local_root).await?;
    let local_root = fs::canonicalize(&requested_root)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Local root is not accessible: {error}")))?;
    payload.local_root = normalized_local_path(&local_root);

    if AiRoom::find_all(&deployment.db().pool)
        .await?
        .iter()
        .any(|room| same_local_root(&room.local_root, &payload.local_root))
    {
        if created_root {
            let _ = fs::remove_dir(&local_root).await;
        }
        return Err(ApiError::Conflict(
            "This local folder is already connected to an AI Room".into(),
        ));
    }

    let room = match AiRoom::create(&deployment.db().pool, payload).await {
        Ok(room) => room,
        Err(error) => {
            if created_root {
                let _ = fs::remove_dir(&local_root).await;
            }
            return Err(error.into());
        }
    };
    Ok(ResponseJson(ApiResponse::success(room)))
}

async fn prepare_local_root(path: &FsPath, allow_existing: bool) -> Result<bool, ApiError> {
    match fs::metadata(path).await {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(ApiError::BadRequest(
                "Local root points to a file, not a directory".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).await.map_err(|error| {
                ApiError::BadRequest(format!("Unable to create local root: {error}"))
            })?;
            return Ok(true);
        }
        Err(error) => {
            return Err(ApiError::BadRequest(format!(
                "Local root is not accessible: {error}"
            )));
        }
    }

    if fs::metadata(path.join(ROOM_DIR).join("room.json"))
        .await
        .is_ok()
    {
        return Err(ApiError::Conflict(
            "This folder already contains another AI Room installation".into(),
        ));
    }

    let mut entries = fs::read_dir(path).await?;
    if entries.next_entry().await?.is_some() && !allow_existing {
        return Err(ApiError::Conflict(format!(
            "{LOCAL_ROOT_NOT_EMPTY_MARKER}: The selected local folder contains existing files"
        )));
    }
    Ok(false)
}

fn same_local_root(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
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

pub async fn get_room_storage(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<AiRoomStorageStatus>>, ApiError> {
    find_room(&deployment, room_id).await?;
    let (identity, profile) = AiRoomStorageProfile::ensure(&deployment.db().pool, room_id).await?;

    Ok(ResponseJson(ApiResponse::success(AiRoomStorageStatus {
        identity,
        profile,
        task_ai_cloud_available: false,
        personal_hub_available: false,
    })))
}

pub async fn initialize_room(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let _guard = SYNC_LOCK.lock().await;
    let room = find_room(&deployment, room_id).await?;
    initialize_local(&room).await?;
    AiRoom::set_instruction_version(&deployment.db().pool, room.id, INSTRUCTION_VERSION).await?;
    let room = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(room).await,
    )))
}

pub async fn update_room_profile(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
    Json(payload): Json<UpdateAiRoomProfileRequest>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let name = payload.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::BadRequest("룸 이름은 1~200자여야 합니다".into()));
    }
    let description = payload
        .description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if description
        .as_deref()
        .is_some_and(|value| value.chars().count() > 2000)
    {
        return Err(ApiError::BadRequest(
            "룸 설명은 2000자 이하여야 합니다".into(),
        ));
    }
    // Remote instruction writes must not race the 15-second sync loop, which
    // holds the same lock while rewriting ROOM.md/AGENTS.md/CLAUDE.md.
    let _guard = SYNC_LOCK.lock().await;
    let previous = find_room(&deployment, room_id).await?;
    let updated = AiRoom::update_profile(&deployment.db().pool, room_id, name, description).await?;
    // Rewrite the local ROOM.md / room.json so the new name is durable. If the
    // local rewrite fails, restore the previous profile so the database and
    // the on-disk instruction cannot diverge permanently.
    if let Err(error) = initialize_local(&updated).await {
        let _ = AiRoom::update_profile(
            &deployment.db().pool,
            room_id,
            previous.name,
            previous.description,
        )
        .await;
        return Err(ApiError::BadRequest(format!(
            "로컬 설명서를 다시 쓰지 못해 변경을 되돌렸습니다: {error}"
        )));
    }
    // Push the renamed instructions to a reachable server right away; the
    // 15-second sync would repair it later anyway via the content diff.
    if let (Some(alias), Some(root)) = (&updated.ssh_alias, &updated.remote_root) {
        let remote = read_remote_files(&updated).await;
        if remote.available
            && let Err(error) = upgrade_remote_instructions(
                &updated,
                alias,
                root,
                &remote.files,
            )
            .await
        {
            tracing::warn!("AI Room could not push renamed instructions to the server: {error}");
        }
    }
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(updated).await,
    )))
}

pub async fn update_room_connection(
    State(deployment): State<DeploymentImpl>,
    Path(room_id): Path<Uuid>,
    Json(payload): Json<UpdateAiRoomConnectionRequest>,
) -> Result<ResponseJson<ApiResponse<AiRoomSnapshot>>, ApiError> {
    let _guard = SYNC_LOCK.lock().await;
    let old_room = find_room(&deployment, room_id).await?;
    let ssh_alias = payload
        .ssh_alias
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let remote_root = payload
        .remote_root
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match (&ssh_alias, &remote_root) {
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
    if old_room.ssh_alias == ssh_alias && old_room.remote_root == remote_root {
        return Ok(ResponseJson(ApiResponse::success(
            build_snapshot(old_room).await,
        )));
    }

    if old_room.ssh_alias.is_some() && old_room.remote_root.is_some() {
        let old_remote = read_remote_files(&old_room).await;
        let has_no_active_room = old_remote.error.as_deref() == Some(SERVER_NOT_PREPARED_ERROR);
        if !has_no_active_room
            && let Err(error) = sync_room_internal(&deployment, old_room.clone()).await
            && !payload.force
        {
            return Err(ApiError::BadRequest(format!(
                "기존 서버의 최신 기록을 가져오지 못해 변경을 중단했습니다. 다시 연결하거나 강제 변경을 선택하세요: {error}"
            )));
        }
    }

    let updated = AiRoom::update_connection(
        &deployment.db().pool,
        room_id,
        ssh_alias.clone(),
        remote_root.clone(),
    )
    .await?;
    initialize_local(&updated).await?;
    if let (Some(alias), Some(root)) = (&ssh_alias, &remote_root)
        && let Err(error) = prepare_remote(&updated, alias, root).await
    {
        let restored = AiRoom::update_connection(
            &deployment.db().pool,
            room_id,
            old_room.ssh_alias.clone(),
            old_room.remote_root.clone(),
        )
        .await?;
        let _ = initialize_local(&restored).await;
        return Err(ApiError::BadRequest(format!(
            "새 서버 준비에 실패해 기존 연결로 되돌렸습니다: {error}"
        )));
    }

    // The room now points at a different server, so the cached activity
    // observation from the previous connection no longer applies.
    REMOTE_ACTIVITY.lock().await.remove(&room_id);

    let updated = find_room(&deployment, room_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        build_snapshot(updated).await,
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
    if (!is_session_record_path(&payload.filename)
        && !is_session_conversation_path(&payload.filename))
        || payload.filename.contains(['\\', '\0', '\r', '\n'])
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
    let override_guard = SESSION_OVERRIDE_LOCK.lock().await;
    let mut overrides = read_session_overrides(&room_dir).await;
    if let Some(status) = status {
        overrides.insert(payload.filename, status);
    } else {
        overrides.remove(&payload.filename);
    }
    write_session_overrides(&room_dir, &overrides).await?;
    drop(override_guard);
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
    let removed_from_remote = Vec::new();
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

        // Server rooms are persistent by owner decision (2026-08-06): records
        // stay resident on the server and are never cleaned up automatically.
        // An instruction-upgrade failure must not fail the whole sync — the
        // checkpoint copies above already happened, so report and continue.
        if let Err(error) = upgrade_remote_instructions(
            &room,
            alias,
            root,
            &remote.files,
        )
        .await
        {
            tracing::warn!("AI Room could not upgrade server instructions during sync: {error}");
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
        .filter(|(name, _)| is_session_record_path(name))
    {
        match local.files.get(filename) {
            None => {
                if create_local_session_record(room, filename, content).await? {
                    local.files.insert(filename.clone(), content.clone());
                    copied.push(filename.clone());
                } else {
                    let path = safe_room_path(room, filename)?;
                    let current = fs::read_to_string(path).await?;
                    local.files.insert(filename.clone(), current.clone());
                    if current != *content {
                        conflicts.push(filename.clone());
                    }
                }
            }
            Some(local_content) if local_content == content => {}
            Some(local_content) => {
                // Every session record is immutable, including legacy flat
                // files. Preserve the local body and surface the divergent
                // path instead of replacing history in place.
                preserve_local_version(room, filename, local_content).await?;
                conflicts.push(filename.clone());
            }
        }
    }

    // context.md is owner-authored and edited locally; the local copy is
    // canonical. When the two sides differ, the room synchronization sends
    // the local version to the room's own prepared server workspace so a
    // stale server copy can no longer erase the owner's edits.
    if let (Some(local_context), Some(remote_context)) = (
        local.files.get("context.md"),
        remote.files.get("context.md"),
    ) && local_context != remote_context
        && let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root)
    {
        // Archive the losing server copy exactly like sessions and library
        // documents, so no divergent version is ever lost without a trace.
        preserve_local_version(room, "context.md", remote_context).await?;
        // Detached like refresh_remote_activity: callers hold the global sync
        // lock, so an unreachable host must not stall every room's sync for
        // the SSH timeout. While the sides still differ, the next cycle
        // retries naturally.
        let alias = alias.clone();
        let root = root.clone();
        let update = vec![("context.md".to_string(), local_context.clone())];
        tokio::spawn(async move {
            match write_remote_files(&alias, &root, &update).await {
                Ok(()) => {
                    tracing::info!("AI Room updated the server copy of context.md");
                }
                Err(error) => {
                    tracing::warn!(
                        "AI Room could not update the server copy of context.md: {error}"
                    );
                }
            }
        });
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
    current_files: &BTreeMap<String, String>,
) -> Result<(), ApiError> {
    if room.instruction_version > INSTRUCTION_VERSION {
        return Ok(());
    }
    let instruction = room_instruction(room);
    let manifest = serde_json::json!({
        "room_id": room.id,
        "name": room.name,
        "instruction_version": INSTRUCTION_VERSION,
    });
    let block = managed_agent_block(room);
    let owner_rules = fs::read_to_string(
        PathBuf::from(&room.local_root)
            .join(ROOM_DIR)
            .join(OWNER_RULES_FILE),
    )
    .await
    .unwrap_or_else(|_| owner_working_rules(&[]));
    let adversarial_review = fs::read_to_string(
        PathBuf::from(&room.local_root)
            .join(ROOM_DIR)
            .join(ADVERSARIAL_REVIEW_FILE),
    )
    .await
    .unwrap_or_else(|_| adversarial_code_review_protocol().into());
    let desired = vec![
        (
            "room.json".into(),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ),
        ("ROOM.md".into(), instruction),
        (OWNER_RULES_FILE.into(), owner_rules),
        (ADVERSARIAL_REVIEW_FILE.into(), adversarial_review),
    ];
    let mut files = desired
        .into_iter()
        .filter(|(name, content)| current_files.get(name) != Some(content))
        .collect::<Vec<_>>();
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let existing = current_files
            .get(&format!("project-root/{filename}"))
            .cloned()
            .unwrap_or_default();
        let desired = upsert_managed_block(&existing, &block);
        if desired != existing {
            files.push((filename.into(), desired));
        }
    }
    if files.is_empty() {
        return Ok(());
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
            // The local model dashboard runs a 4B model on this machine's own
            // GPU. It stays idle unless the owner opts in.
            if !local_summary_enabled() {
                continue;
            }
            let _guard = TASK_SUMMARY_LOCK.lock().await;
            if let Err(error) = summarize_pending_sessions(&summary_deployment).await {
                tracing::debug!("AI Room local task summarizer is waiting: {error}");
            }
        }
    });

    tokio::spawn(async move {
        match AiRoom::find_all(&deployment.db().pool).await {
            Ok(rooms) => {
                for room in rooms
                    .into_iter()
                    .filter(|room| room.instruction_version < INSTRUCTION_VERSION)
                {
                    let _guard = SYNC_LOCK.lock().await;
                    if let Err(error) = initialize_local(&room).await {
                        tracing::warn!(
                            room_id = %room.id,
                            "AI Room instructions could not be upgraded: {error}"
                        );
                    } else if let Err(error) = AiRoom::set_instruction_version(
                        &deployment.db().pool,
                        room.id,
                        INSTRUCTION_VERSION,
                    )
                    .await
                    {
                        tracing::warn!(
                            room_id = %room.id,
                            "AI Room instruction version could not be recorded: {error}"
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

            // A local root can be temporarily unavailable at app startup
            // (detached drive, network folder, permission prompt). Keep
            // retrying old registered rooms until the generated v16 files and
            // managed AGENTS/CLAUDE blocks are actually installed.
            for room in rooms
                .iter()
                .filter(|room| room.instruction_version < INSTRUCTION_VERSION)
            {
                let _guard = SYNC_LOCK.lock().await;
                match initialize_local(room).await {
                    Ok(()) => {
                        if let Err(error) = AiRoom::set_instruction_version(
                            &deployment.db().pool,
                            room.id,
                            INSTRUCTION_VERSION,
                        )
                        .await
                        {
                            tracing::warn!(
                                room_id = %room.id,
                                "AI Room automatic instruction upgrade could not be recorded: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            room_id = %room.id,
                            "AI Room automatic local instruction upgrade will retry: {error}"
                        );
                    }
                }
            }

            for room in rooms
                .into_iter()
                .filter(|room| room.ssh_alias.is_some() && room.remote_root.is_some())
            {
                // Runs before the availability gate so activity on a server
                // whose temporary room was already cleaned up (or never
                // prepared) is still observed, and detached so a slow remote
                // scan cannot delay the other rooms' synchronization.
                {
                    let room = room.clone();
                    tokio::spawn(async move { refresh_remote_activity(&room).await });
                }

                let remote = read_remote_files(&room).await;
                if !remote.available {
                    if remote.error.as_deref() == Some(SERVER_NOT_PREPARED_ERROR)
                        && let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root)
                    {
                        match prepare_remote(&room, alias, root).await {
                            Ok(()) => tracing::info!(
                                room_id = %room.id,
                                "AI Room automatically prepared missing server instructions"
                            ),
                            Err(error) => tracing::warn!(
                                room_id = %room.id,
                                "AI Room automatic server preparation will retry: {error}"
                            ),
                        }
                    }
                    continue;
                }

                let _guard = SYNC_LOCK.lock().await;
                let mut local = read_local_files(&room).await;
                match sync_remote_checkpoints(&room, &mut local, &remote).await {
                    Ok((copied, conflicts)) => {
                        if let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root)
                            && let Err(error) = upgrade_remote_instructions(
                                &room,
                                alias,
                                root,
                                &remote.files,
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
                            // Keep the room list ordered by real activity now
                            // that no automatic path calls sync_room_internal.
                            if let Err(error) = AiRoom::touch(&deployment.db().pool, room.id).await
                            {
                                tracing::warn!(
                                    "AI Room could not refresh the room timestamp: {error}"
                                );
                            }
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
    REMOTE_ACTIVITY.lock().await.remove(&room_id);
    LOCAL_ACTIVITY.lock().await.remove(&room_id);
    SUMMARY_BACKOFF.lock().await.remove(&room_id);
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
        "# AI Room: {name}\n\nRoom ID: `{id}`\nInstruction version: {version}{description}\n\n## Required session workflow\n\n1. Before doing any work, locate the nearest `.ai-room/ROOM.md` by checking the current working directory and then each parent directory. The directory containing that `.ai-room` is the room root. This rule still applies when the AI starts inside a nested module or subfolder. If no ancestor contains it, there is no room for this checkout.\n2. From the room root, read `sessions/INDEX.md` first when it exists, then `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, relevant files in `.ai-room/library/`, additional Markdown instructions directly under `.ai-room/`, and the newest session records. Read active (`진행중`) rows before choosing files to edit. Resolve every room path from the room root, not from the current subfolder.\n3. Give this chat window one unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<conversation-id>/` directory. Never use another chat's directory. Create `000001-start.md` inside it before project work, using this header shape: `# Session: one-line title`, `- Agent: agent name`, `- Module: affected module or area`, `- Status: 진행중`, and `- Started: YYYY-MM-DD HH:MM (timezone)`.\n4. Treat every checkpoint file as immutable. Before each user-facing final response and at meaningful transitions, create the next zero-padded file such as `000002-checkpoint.md`; never edit, replace, rename, or delete an earlier checkpoint. Each new file must repeat the header and preserve the new Goal/checkpoint evidence, decisions with approval state, blockers, changed files, verification, and ordered next steps needed for handoff.\n5. While a user-requested task is still running, send a user-visible progress report when work starts and at least once every 5 minutes of wall-clock time until the final response. A session-file checkpoint does not count as a user report. Each report must state what finished, what is running now, any blocker, and what will happen next. Do not repeat a generic waiting message. Split long-running commands or waits where possible so the reporting interval is not missed; if one operation cannot be interrupted for 5 minutes, warn the user before starting it and report immediately when it returns.\n6. Before editing a file, inspect active sessions for overlapping ownership. Do not edit files claimed by another active AI; create your own conversation directory, link the preceding session when continuing its work, and ask the user to coordinate overlaps. If the user pauses or cancels work, create one final checkpoint with `Status: 중단` or `Status: 보류` and record why.\n7. When the user asks you to remember a reusable method, rule, convention, checklist, prompt, or operating procedure, create or update one focused Markdown file in the room root's `.ai-room/library/`. Use a descriptive filename ending in `.md`, keep one topic per file, and make it understandable without chat history. Do not use the library for transient session notes.\n8. Do not edit `tasks.md` or `decisions.md`. Task AI Platform reads stable session checkpoints together and locally rebuilds both documents. Treat `context.md` as owner-authored and edit it only when explicitly asked.\n9. Never store secrets, tokens, private keys, raw credentials, personal data, or generated binaries in room files.\n10. Before the final response, create the next checkpoint so another AI can continue without chat history. Set its `Status` to exactly one of `완료`, `중단`, or `보류`. If `sessions/INDEX.md` documents a local regeneration command, run it before writing that final checkpoint. Add `{complete_marker}` as the final line of the final checkpoint only. Never go back to mark an earlier file complete.\n\n## Server privacy\n\nWhile work is active, Task AI Platform copies changing server session checkpoints to local storage without deleting the server files. Once every remote session is complete and the merge is conflict-free, it removes the temporary server room. Task and decision summarization uses only the local Ollama service; session contents are not sent to a cloud model.\n\n## Room endpoints\n\n- Local root: `{local}`\n- Remote root: `{remote}`\n\nThe Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.\n",
        name = room.name,
        id = room.id,
        version = INSTRUCTION_VERSION,
        description = room
            .description
            .as_deref()
            .map(|value| format!("\nDescription: {value}"))
            .unwrap_or_default(),
        complete_marker = SESSION_COMPLETE_MARKER,
        local = room.local_root,
        remote = room
            .ssh_alias
            .as_ref()
            .zip(room.remote_root.as_ref())
            .map(|(alias, root)| format!("{alias}:{root}"))
            .unwrap_or_else(|| "not configured".into()),
    );
    let instruction = instruction
        .replace("<conversation-id>", "<random-id>")
        .replace(
            "Never use another chat's directory.",
            "Generate at least 8 random hexadecimal characters for `<random-id>` and, if that directory already exists, generate another. Never use another chat's directory.",
        )
        .replace(
            "Split long-running commands or waits where possible so the reporting interval is not missed; if one operation cannot be interrupted for 5 minutes, warn the user before starting it and report immediately when it returns.",
            "Never start a foreground tool call, command, or wait that can block reporting for 4 minutes or longer. Start long work asynchronously or in the background and poll it at intervals of at most 60 seconds so user reports remain possible. If an operation truly cannot be interrupted, warn the user before it starts and report immediately when it returns.",
        )
        .replace(
            "then `.ai-room/context.md`",
            "then `.ai-room/library/owner-working-rules.md`, `.ai-room/context.md`",
        )
        .replace(
            "While work is active, Task AI Platform copies changing server session checkpoints to local storage without deleting the server files. Once every remote session is complete and the merge is conflict-free, it removes the temporary server room.",
            "Server room records are persistent: Task AI Platform never deletes them automatically, and it continuously copies changing server session checkpoints to local storage.",
        )
        .replace(
            "If `sessions/INDEX.md` documents a local regeneration command, run it before writing that final checkpoint. Add `<!-- task-ai-room:complete -->` as the final line of the final checkpoint only. Never go back to mark an earlier file complete.",
            "Add `<!-- task-ai-room:complete -->` as that new file's final line. Never go back to mark an earlier file complete. Then regenerate `sessions/INDEX.md` when its documented command exists without modifying the checkpoint.",
        )
        .replace("## Server privacy", "## Server retention and privacy");
    format!(
        "{instruction}\n## Mandatory adversarial code review\n\nAfter changing code or executable configuration, read `.ai-room/library/{ADVERSARIAL_REVIEW_NAME}` and apply it before claiming completion. Use two independent critics with different roles and preferably different model families. Let tests, compiler output, static analysis, and reproducible traces outrank model consensus. Record the critics, findings, adjudication evidence, rerun verification, and unresolved risk in the session. This does not apply to answer-only, research-only, planning-only, or prose-only work.\n\n## Record language\n\n- Session checkpoint files may use whichever language lets the active AI preserve technical meaning and handoff context most accurately; they do not need to be Korean.\n- `decisions.md` is shared by the owner and every AI. Task AI Platform renders its explanatory text in Korean and translates session content when necessary. Keep code identifiers, file paths, and product names unchanged when translation would damage their meaning.\n"
    )
}

fn managed_agent_block(room: &AiRoom) -> String {
    let _ = room;
    format!(
        "{START_MARKER}\n## Shared AI Room — mandatory\n\nBefore analysis or the first project tool call, search from the current working directory upward for the nearest `.ai-room/ROOM.md`. A room located in a parent project root still applies inside a nested module. If found, its workflow is required, not optional documentation.\n\nUse the directory containing `.ai-room` as the room root:\n1. Read `.ai-room/sessions/INDEX.md` first when present, then `ROOM.md` and the room files it requires. Check active (`진행중`) sessions before choosing files to edit.\n2. Give this chat its own unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<conversation-id>/` directory. Never use another chat's directory.\n3. Before project work, create `000001-start.md` there with this exact header shape: `# Session: title`, `- Agent: name`, `- Module: area`, `- Status: 진행중`, `- Started: YYYY-MM-DD HH:MM (timezone)`.\n\nDuring work:\n- Before every user-facing final response and at meaningful transitions, add the next zero-padded Markdown checkpoint. Never rewrite, rename, or delete an existing checkpoint.\n- Send the user a visible progress report when work starts and at least every 5 minutes until completion. Session-file writes do not count. State what finished, what is running, blockers, and what comes next; do not repeat generic waiting text. Warn before an uninterruptible operation that may exceed 5 minutes and report immediately afterward.\n- Repeat the discoverable header in every checkpoint and record Goal, checkpoint evidence, decisions and approval state, blockers, failed approaches, changed files, verification, and ordered next steps.\n- Do not edit files claimed by another active session without user coordination.\n- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.\n\nBefore the final response, create one final checkpoint with `Status: 완료`, `중단`, or `보류`, regenerate `sessions/INDEX.md` first when its documented command exists, and put the completion marker required by `ROOM.md` on that new file's final line. AI Room records are private runtime data and must never be committed to Git.\n{END_MARKER}"
    )
    .replace("<conversation-id>", "<random-id>")
    .replace(
        "Never use another chat's directory.",
        "Generate at least 8 random hexadecimal characters for `<random-id>` and, if that directory already exists, generate another. Never use another chat's directory.",
    )
    .replace(
        "Warn before an uninterruptible operation that may exceed 5 minutes and report immediately afterward.",
        "Never use one foreground tool call or wait that can block reporting for 4 minutes or longer; run long work asynchronously and poll at most every 60 seconds. Warn before a truly uninterruptible operation and report immediately afterward.",
    )
    .replace(
        "then `ROOM.md` and the room files it requires",
        "then `ROOM.md`, `.ai-room/library/owner-working-rules.md`, and the room files they require",
    )
    .replace(
        "regenerate `sessions/INDEX.md` first when its documented command exists, and put the completion marker required by `ROOM.md` on that new file's final line.",
        "put the completion marker required by `ROOM.md` on that new file's final line. Then regenerate `sessions/INDEX.md` when its documented command exists without modifying the checkpoint.",
    )
    .replace(
        "- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.\n\nBefore the final response",
        &format!("- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.\n- After code or executable-configuration changes, read `.ai-room/library/{ADVERSARIAL_REVIEW_NAME}` and complete its two-independent-critic, evidence-driven review before claiming completion.\n\nBefore the final response"),
    )
}

fn upsert_managed_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end_start)) = (existing.find(START_MARKER), existing.find(END_MARKER))
        && end_start >= start
    {
        let end = end_start + END_MARKER.len();
        return format!("{}{}{}", &existing[..start], block, &existing[end..]);
    }
    if let Some(start) = existing.find(START_MARKER) {
        let preserved = existing.replacen(START_MARKER, "", 1);
        return format!("{}\n\n{block}\n", preserved.trim());
    }
    if existing.contains(END_MARKER) {
        let preserved = existing.replacen(END_MARKER, "", 1);
        return format!("{}\n\n{block}\n", preserved.trim());
    }
    if existing.trim().is_empty() {
        format!("{block}\n")
    } else {
        format!("{}\n\n{block}\n", existing.trim_end())
    }
}

fn ensure_room_gitignore_entry(existing: &str) -> String {
    let already_ignored = existing.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#')
            && !line.starts_with('!')
            && line.trim_start_matches('/').trim_end_matches('/') == ".ai-room"
    });
    if already_ignored {
        return existing.to_string();
    }

    let prefix = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!(
        "{existing}{prefix}\n# AI Room records are private, machine-local runtime data\n{ROOM_GITIGNORE_ENTRY}\n"
    )
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
        (OWNER_RULES_FILE.into(), owner_working_rules(&[])),
        (
            ADVERSARIAL_REVIEW_FILE.into(),
            adversarial_code_review_protocol().into(),
        ),
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

fn owner_working_rules(additional_documents: &[String]) -> String {
    let mut content = String::from(
        "# 프로젝트 소유자의 AI 작업 규칙\n\n이 문서는 사용자가 여러 작업에서 반복해 요구한 방식을 통합한 필수 규칙이다. 모든 AI는 작업을 시작하기 전에 읽고, 프로젝트별 세부 규칙 문서도 함께 따른다.\n\n## 공통 작업 방식\n\n- 사용자의 지시 범위를 임의로 줄이거나 늘리거나 다른 방식으로 대체하지 않는다. 더 나은 방법이나 부수효과가 보이면 코드에 몰래 반영하지 말고 먼저 말한다.\n- 사용자가 만든 설계 문서와 기존 구조를 사양으로 취급한다. 코드를 쓰기 전에 관련 마스터 문서, AI Room 문서, 유사 코드를 읽는다. 구조 변경은 사전에 알리고 승인을 받는다.\n- 사용자가 질문·의견·현실성 검토만 요청한 경우 명시적인 구현 지시 전에는 코딩·설치·실행을 시작하지 않는다.\n- 기본 구현은 장기적으로 유지 가능한 방식을 택한다. 임시방편은 사용자가 명시적으로 요청할 때만 사용한다.\n- 살아 있는 작업 데이터로 테스트하지 않는다. 격리된 사본이나 임시 데이터를 사용하며, 소유하지 않은 세션의 데이터 이동·삭제를 하지 않는다.\n- 커밋·푸시·브랜치 변경 같은 Git 외부 반영은 사용자가 명시적으로 요청한 범위에서만 수행한다. 이미 범위가 명확한 지시를 다시 의심해 시간을 쓰지 않는다.\n- 잘못했을 때 감정적인 사과를 반복하지 말고 원인, 수정 결과, 재발 방지 규칙을 짧게 남긴다.\n- 사용자에게 답변할 때는 항상 존댓말을 사용한다. 사용자가 반말을 쓰더라도 이를 따라 반말로 전환하지 않는다.\n- 사용자를 부를 필요가 있으면 `호명님` 또는 `Homaung`을 사용하고 추측성 호칭을 쓰지 않는다.\n- 사용자는 적녹 색약이므로 빨강과 초록만으로 의미를 구분하지 않는다. 파랑·마젠타·노랑과 모양·문자를 함께 사용한다.\n\n## 코드 변경의 3권 분립 검토\n\n- 코드나 실행 결과에 영향을 주는 설정을 변경하면 완료 응답 전에 [`adversarial-code-review-protocol.md`](adversarial-code-review-protocol.md)를 반드시 적용한다.\n- 메인 구현자와 독립된 기술 감사관·요구사항 감사관이 먼저 서로의 결론을 보지 않고 검토한다.\n- 가능하면 저비용 Codex 계열과 저비용 Claude 계열을 교차 사용한다. 사용할 수 없으면 격리된 두 검토 역할로 대체하고 그 한계를 공개한다.\n- 검토자 간 토론은 충돌한 지적에 대해 한 번만 허용하며, 다수결이나 말의 설득력보다 테스트·컴파일·정적 분석·재현 결과를 우선한다.\n- 확인된 blocker·high·medium 지적을 처리하고 검증을 다시 실행하기 전에는 작업을 완료로 표시하지 않는다.\n\n## 진행 보고와 세션 기록은 별개\n\n- 진행 중에는 사용자 대화에 최대 5분마다 중간 보고한다. 이것은 세션 Markdown 기록 주기가 아니다.\n- 작업 시작 시 대화창마다 고유한 `sessions/YYYYMMDD-HHMMSS-agent-conversation-id/` 폴더를 만들고 첫 체크포인트 파일을 적는다. 다른 대화창의 폴더를 공유하지 않는다.\n- 하나의 사용자 대화/AI 응답 단위가 끝나기 전에 같은 폴더에 다음 순번 Markdown 파일을 새로 추가한다. 기존 파일은 수정·교체·이름 변경·삭제하지 않는다.\n- 세션 파일을 5분마다 기계적으로 만들지 않는다. 사용자에게 보이는 5분 보고와 영속 세션 기록을 서로 대체하지 않는다.\n- 다른 AI의 진행 중 세션과 소유 파일을 먼저 확인하며, 남의 세션 폴더나 파일을 수정하지 않는다.\n\n## 프로젝트별 추가 필수 문서\n\n",
    );
    content = content.replace(
        "- 작업 시작 시 대화창마다 고유한 `sessions/YYYYMMDD-HHMMSS-agent-conversation-id/` 폴더를 만들고 첫 체크포인트 파일을 적는다. 다른 대화창의 폴더를 공유하지 않는다.",
        "- 작업 시작 시 대화창마다 고유한 `sessions/YYYYMMDD-HHMMSS-agent-random-id/` 폴더를 만들고 첫 체크포인트 파일을 적는다. `random-id`는 임의의 16진수 8자 이상이며 이미 존재하는 폴더명은 다시 쓰지 않는다. 다른 대화창의 폴더를 공유하지 않는다.",
    );
    if additional_documents.is_empty() {
        content.push_str("- 현재 발견된 추가 규칙 문서가 없습니다.\n");
    } else {
        for document in additional_documents {
            content.push_str(&format!("- [`{document}`]({document})\n"));
        }
    }
    content
}

fn adversarial_code_review_protocol() -> &'static str {
    "# 3권 분립형 적대 코드 검토 규약\n\n코드나 실행 결과에 영향을 주는 설정을 변경한 작업은 완료 응답 전에 이 규약을 적용한다. 단순 질문, 조사, 기획, 실행에 영향을 주지 않는 문서 수정에는 적용하지 않는다.\n\n## 역할\n\n- **구현자:** 사용자 요구를 구현하고 기본 테스트를 수행한다.\n- **기술 감사관:** 가능하면 저비용 Codex 계열의 독립 에이전트를 사용한다. 정확성, 회귀, API·타입 계약, 동시성, 성능, 마이그레이션과 누락된 테스트를 검토한다.\n- **요구사항 감사관:** 가능하면 저비용 Claude 계열의 독립 에이전트를 사용한다. 사용자 의도 누락, 보안, 개인정보, 권한, 운영 실패, 위험한 기본값과 오해를 부르는 UI를 검토한다.\n\n서로 다른 계열을 함께 사용할 수 없으면 격리된 새 에이전트 두 개를 서로 다른 역할로 사용한다. 서브에이전트를 사용할 수 없으면 같은 AI가 문맥을 분리한 두 번의 검토를 수행하고 그 한계를 공개한다.\n\n## 필수 절차\n\n1. 구현자는 두 감사관에게 동일한 최소 증거 묶음을 제공한다. 사용자 요구, 제약, 변경 diff, 관련 주변 코드, 적용되는 규칙, 이미 실행한 검증 결과와 알려진 한계를 포함한다.\n2. 두 감사관은 첫 검토에서 서로의 결과와 구현자의 결론을 보지 않는다. 검토 중 실제 프로젝트 파일을 수정하지 않는다.\n3. 모든 지적은 심각도, 정확한 파일·좁은 줄 범위, 실패 시나리오, 재현 방법 또는 판별 테스트, 최소 수정안을 포함해야 한다.\n4. 두 결과가 충돌하거나 겹칠 때만 서로의 지적과 증거를 보여주고 각각 한 번만 반박하게 한다. 무제한 토론과 다수결은 금지한다.\n5. 판정 우선순위는 재현 테스트, 런타임 추적, 컴파일·타입 검사·정적 분석, 사용자 요구·프로젝트 규칙, 공식 계약, 모델 주장 순서다.\n6. 확인된 blocker·high·medium 지적을 수정하고 관련 검증을 다시 실행한다. 남은 위험을 조용히 무시하지 않는다.\n7. 고위험 변경에서 해결되지 않은 충돌은 강한 독립 판정자나 사용자에게 승격한다.\n\n## 완료 조건\n\n세션 기록과 사용자 완료 보고에 감사관 또는 대체 방식, 위험 등급, 확인·기각·수정·수용한 지적, 판정 증거, 수정 후 검증, 해결되지 않은 위험과 승격 상태를 남긴다. 이 항목이 없으면 코드 변경 작업을 완료로 표시하지 않는다.\n"
}

fn is_session_record_name(name: &str) -> bool {
    name.ends_with(".md") && !name.eq_ignore_ascii_case(SESSION_INDEX_FILE)
}

fn is_session_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.'))
        && name != "."
        && name != ".."
}

fn is_session_record_path(path: &str) -> bool {
    let Some(relative) = path.strip_prefix("sessions/") else {
        return false;
    };
    let parts = relative.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        [filename] => is_session_component(filename) && is_session_record_name(filename),
        [conversation, filename] => {
            is_session_component(conversation)
                && !is_session_record_name(conversation)
                && is_session_component(filename)
                && is_session_record_name(filename)
        }
        _ => false,
    }
}

fn session_conversation_key(path: &str) -> Option<String> {
    let relative = path.strip_prefix("sessions/")?;
    let mut parts = relative.split('/');
    let first = parts.next()?;
    match (parts.next(), parts.next()) {
        (None, None) if is_session_record_path(path) => Some(path.to_string()),
        (Some(_), None) if is_session_record_path(path) => Some(format!("sessions/{first}")),
        _ => None,
    }
}

fn is_session_conversation_path(path: &str) -> bool {
    let Some(relative) = path.strip_prefix("sessions/") else {
        return false;
    };
    !relative.contains('/')
        && is_session_component(relative)
        && !is_session_record_name(relative)
}

fn aggregate_session_records(
    local: &BTreeMap<String, String>,
    remote: &BTreeMap<String, String>,
) -> (Vec<AiRoomRecord>, Vec<String>) {
    let mut groups = BTreeMap::<String, Vec<(String, String, String)>>::new();
    let mut conflicts = Vec::new();
    let mut names = local
        .keys()
        .chain(remote.keys())
        .filter(|name| is_session_record_path(name))
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();

    for filename in names {
        let Some(conversation) = session_conversation_key(&filename) else {
            continue;
        };
        let (content, source) = match (local.get(&filename), remote.get(&filename)) {
            (Some(left), Some(right)) if left == right => (left.clone(), "both".to_string()),
            (Some(left), Some(_)) => {
                conflicts.push(filename.clone());
                (left.clone(), "conflict".to_string())
            }
            (Some(content), None) => (content.clone(), "local".to_string()),
            (None, Some(content)) => (content.clone(), "remote".to_string()),
            _ => continue,
        };
        groups
            .entry(conversation)
            .or_default()
            .push((filename, content, source));
    }

    let mut records = groups
        .into_iter()
        .map(|(filename, mut checkpoints)| {
            checkpoints.sort_by(|left, right| left.0.cmp(&right.0));
            let source = if checkpoints.iter().all(|entry| entry.2 == "both") {
                "both"
            } else if checkpoints.iter().all(|entry| entry.2 == "local") {
                "local"
            } else if checkpoints.iter().all(|entry| entry.2 == "remote") {
                "remote"
            } else if checkpoints.iter().all(|entry| entry.2 == "conflict") {
                "conflict"
            } else {
                "mixed"
            };
            let content = if checkpoints.len() == 1 && checkpoints[0].0 == filename {
                checkpoints.remove(0).1
            } else {
                checkpoints
                    .into_iter()
                    .map(|(checkpoint, content, _)| {
                        let name = checkpoint.rsplit('/').next().unwrap_or(&checkpoint);
                        format!("<!-- AI Room checkpoint: {name} -->\n\n{content}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n")
            };
            AiRoomRecord {
                filename,
                content,
                source: source.into(),
            }
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| right.filename.cmp(&left.filename));
    (records, conflicts)
}

fn session_is_complete(content: &str) -> bool {
    content
        .lines()
        .next_back()
        .is_some_and(|line| line.trim() == SESSION_COMPLETE_MARKER)
}

/// Newest file modification time under `root`, skipping room records, version
/// control, and generated directories. Bounded by entry count and depth so the
/// scan stays cheap on large project roots.
fn latest_workspace_activity(root: &FsPath) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut visited = 0usize;
    // Breadth-first order spreads the entry budget across sibling directories
    // instead of letting one huge subtree consume it all.
    let mut stack = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = stack.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > ACTIVITY_SCAN_MAX_ENTRIES {
                return newest;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if depth < ACTIVITY_SCAN_MAX_DEPTH
                    && !ACTIVITY_EXCLUDED_DIRS.contains(&name.as_str())
                {
                    stack.push_back((entry.path(), depth + 1));
                }
            } else if file_type.is_file() {
                // The app itself rewrites the managed root instruction files
                // (AGENTS.md/CLAUDE.md) on startup and upgrades; counting them
                // would raise a self-inflicted activity warning.
                if depth == 0 {
                    let name = entry.file_name();
                    if name == "AGENTS.md" || name == "CLAUDE.md" {
                        continue;
                    }
                }
                if let Ok(metadata) = entry.metadata()
                    && let Ok(modified) = metadata.modified()
                    && newest.is_none_or(|current| modified > current)
                {
                    newest = Some(modified);
                }
            }
        }
    }
    newest
}

/// Work is "unrecorded" when the workspace changed recently but no session
/// record (active or completed) was updated within the checkpoint deadline.
fn is_unrecorded_activity(
    activity_age: Option<Duration>,
    latest_record_age: Option<Duration>,
) -> bool {
    activity_age.is_some_and(|age| age <= ACTIVITY_RECENT_WINDOW)
        && latest_record_age.is_none_or(|age| age >= CHECKPOINT_OVERDUE_AFTER)
}

/// Refresh the cached newest remote workspace modification time for a room.
/// Piggybacks on the auto-sync loop and throttles the extra `find` to once per
/// `REMOTE_ACTIVITY_CHECK_INTERVAL`. Failures are cached as "no activity" so an
/// unreachable server does not retry every cycle or raise false warnings.
async fn refresh_remote_activity(room: &AiRoom) {
    let (Some(alias), Some(root)) = (&room.ssh_alias, &room.remote_root) else {
        return;
    };
    let now = SystemTime::now();
    // Reserve the slot before the SSH call so concurrent refreshes for the
    // same room cannot start a duplicate remote scan.
    let previous = {
        let mut cache = REMOTE_ACTIVITY.lock().await;
        let entry = cache.get(&room.id).copied();
        if let Some((checked, _)) = entry
            && now
                .duration_since(checked)
                .is_ok_and(|age| age < REMOTE_ACTIVITY_CHECK_INTERVAL)
        {
            return;
        }
        let previous = entry.and_then(|(_, latest)| latest);
        cache.insert(room.id, (now, previous));
        previous
    };
    let prunes = ACTIVITY_EXCLUDED_DIRS
        .iter()
        .map(|name| format!("-name {}", posix_quote(name)))
        .collect::<Vec<_>>()
        .join(" -o ");
    let script = format!(
        "root={root}; cd \"$root\" 2>/dev/null || exit 2; \
find . -maxdepth {depth} \\( {prunes} \\) -prune -o -type f -mmin -{window} -not -path ./AGENTS.md -not -path ./CLAUDE.md -printf '%T@\\n' 2>/dev/null | sort -rn | head -n 1",
        root = posix_quote(root),
        depth = ACTIVITY_SCAN_MAX_DEPTH,
        window = ACTIVITY_RECENT_WINDOW.as_secs() / 60,
    );
    // A successful command with empty output means "no recent activity"; a
    // failed command keeps the last known value instead of erasing it.
    let latest = match run_remote(alias, &script).await {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|epoch| *epoch > 0.0)
            .map(|epoch| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(epoch)),
        _ => previous,
    };
    REMOTE_ACTIVITY
        .lock()
        .await
        .insert(room.id, (SystemTime::now(), latest));
}

/// Cached wrapper around the local workspace scan so 15-second snapshot
/// polling does not rescan the tree on every request.
async fn cached_local_activity(room: &AiRoom) -> Option<SystemTime> {
    let now = SystemTime::now();
    {
        let cache = LOCAL_ACTIVITY.lock().await;
        if let Some((checked, latest)) = cache.get(&room.id)
            && now
                .duration_since(*checked)
                .is_ok_and(|age| age < LOCAL_ACTIVITY_CHECK_INTERVAL)
        {
            return *latest;
        }
    }
    let root = PathBuf::from(&room.local_root);
    let latest = tokio::task::spawn_blocking(move || latest_workspace_activity(&root))
        .await
        .ok()
        .flatten();
    LOCAL_ACTIVITY
        .lock()
        .await
        .insert(room.id, (SystemTime::now(), latest));
    latest
}

async fn session_record_modified(room: &AiRoom, session: &str) -> Option<SystemTime> {
    let path = PathBuf::from(&room.local_root)
        .join(ROOM_DIR)
        .join(session);
    if is_session_record_path(session) {
        return fs::metadata(path)
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok());
    }
    if !is_session_conversation_path(session) {
        return None;
    }
    let mut newest = None;
    let mut entries = fs::read_dir(path).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = format!("{session}/{name}");
        if !is_session_record_path(&relative) {
            continue;
        }
        if let Ok(metadata) = entry.metadata().await
            && let Ok(modified) = metadata.modified()
            && newest.is_none_or(|current| modified > current)
        {
            newest = Some(modified);
        }
    }
    newest
}

async fn checkpoint_health(
    room: &AiRoom,
    sessions: &[AiRoomRecord],
    overrides: &BTreeMap<String, String>,
) -> AiRoomCheckpointHealth {
    let active = sessions
        .iter()
        .filter(|session| {
            !session_is_complete(&session.content)
                && !session_status(&session.content)
                    .is_some_and(|status| status_is_inactive(&status))
                && !overrides
                    .get(&session.filename)
                    .is_some_and(|status| status_is_stopped(status))
        })
        .collect::<Vec<_>>();
    let now = SystemTime::now();
    let mut ages = Vec::new();
    let mut latest_record_age: Option<Duration> = None;
    for session in sessions {
        if let Some(modified) = session_record_modified(room, &session.filename).await
            && let Ok(age) = now.duration_since(modified)
        {
            if latest_record_age.is_none_or(|current| age < current) {
                latest_record_age = Some(age);
            }
            if active
                .iter()
                .any(|active| active.filename == session.filename)
            {
                ages.push((session.filename.clone(), age));
            }
        }
    }
    ages.sort_by_key(|(_, age)| *age);

    let local_activity = cached_local_activity(room).await;
    let remote_activity = REMOTE_ACTIVITY
        .lock()
        .await
        .get(&room.id)
        .and_then(|(_, activity)| *activity);
    let local_activity_age = local_activity.and_then(|at| now.duration_since(at).ok());
    let remote_activity_age = remote_activity.and_then(|at| now.duration_since(at).ok());
    let newest_activity_age = match (local_activity_age, remote_activity_age) {
        (Some(local), Some(remote)) => Some(local.min(remote)),
        (local, remote) => local.or(remote),
    };
    let unrecorded_activity = is_unrecorded_activity(newest_activity_age, latest_record_age);
    let overdue_sessions = ages
        .iter()
        .filter(|(_, age)| *age >= CHECKPOINT_OVERDUE_AFTER)
        .count();
    let status = if active.is_empty() {
        "idle"
    } else if ages.is_empty() {
        "unknown"
    } else if ages
        .first()
        .is_some_and(|(_, age)| *age >= CHECKPOINT_OVERDUE_AFTER)
    {
        "overdue"
    } else {
        "healthy"
    };
    AiRoomCheckpointHealth {
        status: status.into(),
        active_sessions: active.len(),
        overdue_sessions,
        latest_session: ages.first().map(|(filename, _)| filename.clone()),
        latest_checkpoint_age_seconds: ages
            .first()
            .map(|(_, age)| age.as_secs().min(u32::MAX as u64) as u32),
        overdue_after_minutes: (CHECKPOINT_OVERDUE_AFTER.as_secs() / 60) as u32,
        unrecorded_activity,
        local_activity_age_seconds: local_activity_age
            .map(|age| age.as_secs().min(u32::MAX as u64) as u32),
        remote_activity_age_seconds: remote_activity_age
            .map(|age| age.as_secs().min(u32::MAX as u64) as u32),
    }
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

async fn merge_remote_library_documents(
    room: &AiRoom,
    local_files: &mut BTreeMap<String, String>,
    remote_files: &BTreeMap<String, String>,
) -> Result<(Vec<String>, Vec<String>), ApiError> {
    let remote_documents = remote_library_documents(remote_files);
    let mut copied = Vec::new();
    let conflicts = Vec::new();

    for (filename, remote_content) in &remote_documents {
        match local_files.get(filename) {
            None => {
                write_local_file(room, &filename, remote_content).await?;
                local_files.insert(filename.clone(), remote_content.clone());
                copied.push(filename.clone());
            }
            Some(local_content) if local_content == remote_content => {}
            Some(local_content) => {
                preserve_local_version(room, filename, local_content).await?;
                write_local_file(room, filename, remote_content).await?;
                local_files.insert(filename.clone(), remote_content.clone());
                copied.push(filename.clone());
            }
        }
    }

    Ok((copied, conflicts))
}

fn ensure_task_update_section(content: &str) -> String {
    let dashboard_comment = "<!-- 로컬 요약(TASK_AI_LOCAL_SUMMARY=1)을 켠 동안에만 Task AI Platform가 이 대시보드를 다시 씁니다. 꺼져 있으면 아래 내용은 마지막 갱신 시점 그대로입니다. -->";
    let normalized = content
        .replace(
            "<!-- The local task summarizer appends one validated line per completed session. -->",
            dashboard_comment,
        )
        .replace(
            "<!-- AI agents append exactly one concise line per session below. -->",
            dashboard_comment,
        )
        .replace(
            "<!-- Task AI Platform가 안정된 AI 작업 기록을 바탕으로 이 대시보드를 자동 갱신합니다. -->",
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

/// The local model dashboard is opt-in. It runs a 4B model on the machine's own
/// GPU, so it stays off unless the owner asks for it with
/// `TASK_AI_LOCAL_SUMMARY=1`.
fn local_summary_enabled() -> bool {
    env::var(LOCAL_SUMMARY_ENV).is_ok_and(|value| {
        let value = value.trim();
        value.eq_ignore_ascii_case("1")
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("on")
    })
}

/// Doubles the wait after each consecutive failure so a room that can never
/// produce a valid dashboard stops occupying the GPU.
fn summary_retry_delay(failures: u32) -> Duration {
    let steps = failures.saturating_sub(1).min(16);
    let seconds = SUMMARY_RETRY_BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64 << steps);
    Duration::from_secs(seconds.min(SUMMARY_RETRY_BACKOFF_MAX.as_secs()))
}

async fn summarize_pending_sessions(deployment: &DeploymentImpl) -> Result<(), String> {
    let rooms = AiRoom::find_all(&deployment.db().pool)
        .await
        .map_err(|error| error.to_string())?;
    let mut first_error = None;

    for room in rooms {
        let now = SystemTime::now();
        if let Some((_, next_attempt)) = SUMMARY_BACKOFF.lock().await.get(&room.id).copied()
            && now < next_attempt
        {
            continue;
        }

        match summarize_next_session(&room).await {
            Ok(()) => {
                SUMMARY_BACKOFF.lock().await.remove(&room.id);
            }
            Err(error) => {
                let mut backoff = SUMMARY_BACKOFF.lock().await;
                let failures = backoff
                    .get(&room.id)
                    .map_or(1, |(failures, _)| failures.saturating_add(1));
                let delay = summary_retry_delay(failures);
                backoff.insert(room.id, (failures, now + delay));
                drop(backoff);
                tracing::warn!(
                    room_id = %room.id,
                    failures,
                    retry_in_seconds = delay.as_secs(),
                    "AI Room dashboard rebuild failed; backing off: {error}"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
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
    let local = read_local_files(room).await;
    let (records, _) = aggregate_session_records(&local.files, &BTreeMap::new());
    for record in records {
        let modified = session_record_modified(room, &record.filename).await;
        if session_is_ready_for_summary(&record.content, modified, SystemTime::now()) {
            sessions.push((record.filename, record.content));
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
    let lines = content.lines().collect::<Vec<_>>();
    // Conversation folders concatenate immutable checkpoints oldest-to-newest.
    // Read from the end so the newest checkpoint owns the current status.
    for (index, line) in lines.iter().enumerate().rev() {
        let trimmed = line.trim().trim_start_matches(['-', '*']).trim();
        for prefix in ["Status:", "상태:"] {
            if let Some(status) = trimmed.strip_prefix(prefix) {
                let status = clean_summary_field(status, 100, "").to_lowercase();
                if !status.is_empty() {
                    return Some(status);
                }
            }
        }
        if trimmed.eq_ignore_ascii_case("## Status") || trimmed == "## 상태" {
            return lines[index + 1..]
                .iter()
                .find(|candidate| !candidate.trim().is_empty())
                .map(|status| {
                    clean_summary_field(status.trim().trim_start_matches(['-', '*']), 100, "")
                        .to_lowercase()
                });
        }
    }
    None
}

fn status_is_stopped(status: &str) -> bool {
    status_starts_with(
        status,
        &["stopped", "cancelled", "canceled", "중단", "취소"],
    )
}

fn status_is_inactive(status: &str) -> bool {
    status_is_stopped(status)
        || status_starts_with(
            status,
            &[
                "complete",
                "completed",
                "done",
                "완료",
                "보류",
                "hold",
                "paused",
            ],
        )
}

fn status_starts_with(status: &str, markers: &[&str]) -> bool {
    let normalized = status
        .trim()
        .trim_start_matches(['*', '_', '`'])
        .trim_start();
    markers.iter().any(|marker| normalized.starts_with(marker))
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
        || session_status(content).is_some_and(|status| status_is_inactive(&status))
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
            output.push_str(&format!("- ⏳ 진행: {title}\n"));
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
            output.push_str(&format!("- ✅ 완료: {title}\n"));
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
    if room.instruction_version > INSTRUCTION_VERSION {
        return Ok(());
    }
    let local_root = PathBuf::from(&room.local_root);
    let gitignore_path = local_root.join(".gitignore");
    let gitignore = fs::read_to_string(&gitignore_path)
        .await
        .unwrap_or_default();
    let updated_gitignore = ensure_room_gitignore_entry(&gitignore);
    if updated_gitignore != gitignore {
        fs::write(gitignore_path, updated_gitignore).await?;
    }

    let room_dir = local_root.join(ROOM_DIR);
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
    let additional_rules = discover_owner_rule_documents(&room_dir).await?;
    fs::write(
        room_dir.join(OWNER_RULES_FILE),
        owner_working_rules(&additional_rules),
    )
    .await?;
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
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(room_dir.join(SESSION_OVERRIDES_FILE))
        .await
    {
        Ok(mut file) => {
            file.write_all(b"{}\n").await?;
            file.flush().await?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let block = managed_agent_block(room);
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let path = PathBuf::from(&room.local_root).join(filename);
        if fs::symlink_metadata(&path)
            .await
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(ApiError::BadRequest(format!(
                "Cannot install AI Room instructions because {filename} is a symbolic link"
            )));
        }
        let existing = fs::read_to_string(&path).await.unwrap_or_default();
        fs::write(path, upsert_managed_block(&existing, &block)).await?;
    }
    Ok(())
}

async fn discover_owner_rule_documents(room_dir: &FsPath) -> Result<Vec<String>, ApiError> {
    let mut documents = Vec::new();
    let mut entries = fs::read_dir(room_dir.join(LIBRARY_DIR)).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.eq_ignore_ascii_case(OWNER_RULES_NAME) || !valid_library_filename(&name) {
            continue;
        }
        let normalized = name.to_ascii_lowercase();
        if [
            "rule",
            "instruction",
            "agreement",
            "protocol",
            "convention",
            "standard",
            "procedure",
            "organization",
            "color",
        ]
        .iter()
        .any(|keyword| normalized.contains(keyword))
        {
            documents.push(name);
        }
    }
    documents.sort();
    Ok(documents)
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
        .any(|name| is_session_record_path(name))
    {
        upgrade_remote_instructions(
            room,
            alias,
            root,
            &existing_remote.files,
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
    for (relative, content) in local
        .files
        .iter()
        .filter(|(relative, _)| is_session_record_path(relative))
    {
        files.push((relative.clone(), content.clone()));
    }
    if let Some(content) = local.files.get(SESSION_OVERRIDES_FILE) {
        files.push((SESSION_OVERRIDES_FILE.into(), content.clone()));
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

async fn create_local_session_record(
    room: &AiRoom,
    relative: &str,
    content: &str,
) -> Result<bool, ApiError> {
    if !is_session_record_path(relative) {
        return Err(ApiError::BadRequest("Invalid session record path".into()));
    }
    let path = safe_room_path(room, relative)?;
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::BadRequest("Invalid session record path".into()))?;
    fs::create_dir_all(parent).await?;
    let temporary = parent.join(format!(".task-ai-tmp-{}", Uuid::new_v4()));
    let write_result = async {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary).await;
        return Err(error.into());
    }
    match fs::hard_link(&temporary, &path).await {
        Ok(()) => {
            let _ = fs::remove_file(&temporary).await;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary).await;
            Ok(false)
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            Err(error.into())
        }
    }
}

async fn preserve_local_version(
    room: &AiRoom,
    relative: &str,
    content: &str,
) -> Result<(), ApiError> {
    // Validate the original room-relative path before reusing it below the
    // private history directory. History is deliberately outside the files
    // discovered by read_local_files, so it cannot re-enter synchronization.
    safe_room_path(room, relative)?;
    let history_path = PathBuf::from(&room.local_root)
        .join(ROOM_DIR)
        .join(LOCAL_HISTORY_DIR)
        .join(content_hash(content))
        .join(relative);
    if fs::metadata(&history_path).await.is_ok() {
        return Ok(());
    }
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(history_path, content).await?;
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
    if files
        .iter()
        .any(|(relative, _)| relative == "AGENTS.md" || relative == "CLAUDE.md")
    {
        script.push_str(" if [ -L \"$root/AGENTS.md\" ] || [ -L \"$root/CLAUDE.md\" ]; then echo 'Refusing to replace symbolic-link AI instructions' >&2; exit 4; fi;");
    }
    for (index, (relative, content)) in files.iter().enumerate() {
        if content.len() > MAX_DOCUMENT_BYTES {
            return Err(ApiError::PayloadTooLarge);
        }
        let target = if relative == "AGENTS.md" || relative == "CLAUDE.md" {
            format!("$root/{relative}")
        } else {
            format!("$root/{ROOM_DIR}/{relative}")
        };
        if relative == "AGENTS.md" || relative == "CLAUDE.md" {
            script.push_str(&format!(
                " if [ -L \"{target}\" ]; then echo 'Refusing to replace symbolic link: {relative}' >&2; exit 4; fi;"
            ));
        }
        let parent = FsPath::new(relative)
            .parent()
            .and_then(FsPath::to_str)
            .unwrap_or("");
        if !parent.is_empty() {
            script.push_str(&format!(
                " mkdir -p \"$root/{ROOM_DIR}/{parent}\" || exit 1;"
            ));
        }
        let encoded = posix_quote(&BASE64.encode(content));
        if is_session_record_path(relative) {
            let temporary = format!("{target}.task-ai-tmp-$${index}");
            script.push_str(&format!(
                " printf '%s' {encoded} | base64 -d > \"{temporary}\" || exit 1; if ln \"{temporary}\" \"{target}\" 2>/dev/null; then rm -f \"{temporary}\"; elif cmp -s \"{temporary}\" \"{target}\"; then rm -f \"{temporary}\"; else rm -f \"{temporary}\"; echo 'Session checkpoint already exists with different content: {relative}' >&2; exit 3; fi;"
            ));
        } else {
            script.push_str(&format!(
                " printf '%s' {encoded} | base64 -d > \"{target}\" || exit 1;"
            ));
        }
    }
    let output = run_remote_with_stdin(alias, script.as_bytes()).await?;
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

async fn run_remote_with_stdin(
    alias: &str,
    input: &[u8],
) -> Result<std::process::Output, ApiError> {
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
        "sh -s".into(),
    ];
    run_ssh_with_stdin(&args, input, Duration::from_secs(90)).await
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
    if let Ok(content) = fs::read_to_string(room_dir.join(SESSION_OVERRIDES_FILE)).await {
        result.files.insert(SESSION_OVERRIDES_FILE.into(), content);
    }
    if let Ok(mut entries) = fs::read_dir(room_dir.join("sessions")).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_session_record_name(&name)
                && let Ok(content) = fs::read_to_string(entry.path()).await
            {
                if content.len() > MAX_DOCUMENT_BYTES {
                    result.error.get_or_insert_with(|| {
                        format!("Session checkpoint exceeds the size limit: sessions/{name}")
                    });
                } else {
                    result.files.insert(format!("sessions/{name}"), content);
                }
            } else if is_session_component(&name)
                && let Ok(file_type) = entry.file_type().await
                && file_type.is_dir()
                && let Ok(mut checkpoints) = fs::read_dir(entry.path()).await
            {
                while let Ok(Some(checkpoint)) = checkpoints.next_entry().await {
                    let checkpoint_name = checkpoint.file_name().to_string_lossy().into_owned();
                    let relative = format!("sessions/{name}/{checkpoint_name}");
                    if is_session_record_path(&relative)
                        && let Ok(checkpoint_type) = checkpoint.file_type().await
                        && checkpoint_type.is_file()
                        && let Ok(content) = fs::read_to_string(checkpoint.path()).await
                    {
                        if content.len() > MAX_DOCUMENT_BYTES {
                            result.error.get_or_insert_with(|| {
                                format!("Session checkpoint exceeds the size limit: {relative}")
                            });
                        } else {
                            result.files.insert(relative, content);
                        }
                    }
                }
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
for name in room.json ROOM.md context.md decisions.md tasks.md {session_overrides} {baseline}; do
  file="$room/$name"; [ -f "$file" ] || continue
  printf '%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
done
for name in AGENTS.md CLAUDE.md; do
  file="$root/$name"; [ -f "$file" ] || continue
  printf 'project-root/%s\t' "$name"; base64 "$file" | tr -d '\n'; printf '\n'
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
if [ -d "$room/sessions" ]; then
  find "$room/sessions" -mindepth 1 -maxdepth 2 -type f -name '*.md' | sort | while IFS= read -r file; do
    relative=${{file#"$room/"}}
    [ "$relative" = "sessions/{session_index}" ] && continue
    printf '%s\t' "$relative"; base64 "$file" | tr -d '\n'; printf '\n'
  done
fi"#,
        root = posix_quote(root),
        room_dir = ROOM_DIR,
        session_overrides = SESSION_OVERRIDES_FILE,
        baseline = LIBRARY_BASELINE_FILE,
        session_index = SESSION_INDEX_FILE,
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
                SERVER_NOT_PREPARED_ERROR.into()
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
        {
            if content.len() > MAX_DOCUMENT_BYTES {
                result.error.get_or_insert_with(|| {
                    format!("Remote room record exceeds the size limit: {filename}")
                });
                continue;
            }
            if matches!(
                filename,
                "room.json" | "ROOM.md" | "context.md" | "decisions.md" | "tasks.md"
            ) || filename == SESSION_OVERRIDES_FILE
                || filename == LIBRARY_BASELINE_FILE
                || is_session_record_path(filename)
                || filename
                    .strip_prefix("project-root/")
                    .is_some_and(|name| matches!(name, "AGENTS.md" | "CLAUDE.md"))
                || filename
                    .strip_prefix("library/")
                    .or_else(|| filename.strip_prefix("root-documents/"))
                    .is_some_and(valid_library_filename)
            {
                result.files.insert(filename.to_string(), content);
            }
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
    let (sessions, conflicts) = aggregate_session_records(&local.files, &remote.files);
    let remote_library = remote_library_documents(&remote.files);
    let mut library = Vec::new();
    for filename in ["ROOM.md", "context.md"] {
        match (local.files.get(filename), remote.files.get(filename)) {
            (Some(left), Some(right)) if left == right => library.push(AiRoomRecord {
                filename: filename.into(),
                content: left.clone(),
                source: "both".into(),
            }),
            (Some(left), Some(right)) => library.push(AiRoomRecord {
                filename: filename.into(),
                content: if filename == "ROOM.md" {
                    left.clone()
                } else {
                    right.clone()
                },
                source: if filename == "ROOM.md" {
                    "local".into()
                } else {
                    "remote".into()
                },
            }),
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
            (Some(left), Some(right)) => library.push(AiRoomRecord {
                filename,
                content: if left == right {
                    left.clone()
                } else {
                    right.clone()
                },
                source: if left == right { "both" } else { "remote" }.into(),
            }),
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
    let remote_value = |name: &str| {
        remote
            .files
            .get(name)
            .or_else(|| local.files.get(name))
            .cloned()
            .unwrap_or_default()
    };
    let session_overrides =
        read_session_overrides(&PathBuf::from(&room.local_root).join(ROOM_DIR)).await;
    let checkpoint_health = checkpoint_health(&room, &sessions, &session_overrides).await;
    let managed_block = managed_agent_block(&room);
    let local_managed = {
        let mut installed = true;
        for filename in ["AGENTS.md", "CLAUDE.md"] {
            let content = fs::read_to_string(PathBuf::from(&room.local_root).join(filename))
                .await
                .unwrap_or_default();
            installed &= content.contains(&managed_block);
        }
        installed
    };
    let remote_managed = ["AGENTS.md", "CLAUDE.md"].iter().all(|filename| {
        remote
            .files
            .get(&format!("project-root/{filename}"))
            .is_some_and(|content| content.contains(&managed_block))
    });
    AiRoomSnapshot {
        instruction: value("ROOM.md"),
        context: remote_value("context.md"),
        decisions: value("decisions.md"),
        tasks: value("tasks.md"),
        sessions,
        checkpoint_health,
        local_summary_enabled: local_summary_enabled(),
        session_overrides,
        library,
        conflicts,
        local: AiRoomEndpointState {
            configured: true,
            available: local.available,
            instruction_installed: local.files.get("ROOM.md") == Some(&room_instruction(&room))
                && local_managed,
            error: local.error,
        },
        remote: AiRoomEndpointState {
            configured: room.ssh_alias.is_some(),
            available: remote.available,
            instruction_installed: remote.files.get("ROOM.md") == Some(&room_instruction(&room))
                && remote_managed,
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
        .route("/ai-rooms/{room_id}/storage", get(get_room_storage))
        .route("/ai-rooms/{room_id}/profile", put(update_room_profile))
        .route(
            "/ai-rooms/{room_id}/connection",
            put(update_room_connection),
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
    fn workspace_activity_scan_skips_excluded_directories() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        std::fs::write(root.path().join(".git").join("HEAD"), "ref").unwrap();
        std::fs::create_dir_all(root.path().join(ROOM_DIR).join("sessions")).unwrap();
        std::fs::write(
            root.path().join(ROOM_DIR).join("sessions").join("a.md"),
            "session",
        )
        .unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "managed block").unwrap();
        std::fs::write(root.path().join("CLAUDE.md"), "managed block").unwrap();
        assert!(latest_workspace_activity(root.path()).is_none());

        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src").join("main.rs"), "fn main() {}").unwrap();
        assert!(latest_workspace_activity(root.path()).is_some());
    }

    #[test]
    fn unrecorded_activity_requires_recent_work_and_stale_records() {
        let fresh = Some(Duration::from_secs(60));
        let stale_record = Some(CHECKPOINT_OVERDUE_AFTER + Duration::from_secs(1));
        let old_activity = Some(ACTIVITY_RECENT_WINDOW + Duration::from_secs(1));

        assert!(is_unrecorded_activity(fresh, None));
        assert!(is_unrecorded_activity(fresh, stale_record));
        assert!(!is_unrecorded_activity(fresh, fresh));
        assert!(!is_unrecorded_activity(old_activity, None));
        assert!(!is_unrecorded_activity(None, None));
        assert!(is_unrecorded_activity(
            Some(ACTIVITY_RECENT_WINDOW),
            Some(CHECKPOINT_OVERDUE_AFTER)
        ));
        assert!(!is_unrecorded_activity(
            Some(ACTIVITY_RECENT_WINDOW + Duration::from_secs(1)),
            None
        ));
    }

    #[test]
    fn summary_retry_backs_off_and_stays_capped() {
        assert_eq!(summary_retry_delay(1), SUMMARY_RETRY_BACKOFF_BASE);
        assert_eq!(summary_retry_delay(2), SUMMARY_RETRY_BACKOFF_BASE * 2);
        assert_eq!(summary_retry_delay(3), SUMMARY_RETRY_BACKOFF_BASE * 4);
        // Every later failure is clamped, so a room that can never converge
        // never returns to the 45 second cadence that pinned the GPU.
        assert_eq!(summary_retry_delay(9), SUMMARY_RETRY_BACKOFF_MAX);
        assert_eq!(summary_retry_delay(u32::MAX), SUMMARY_RETRY_BACKOFF_MAX);
        assert!(summary_retry_delay(1) > TASK_SUMMARY_INTERVAL);
    }

    #[test]
    fn local_summary_stays_off_unless_explicitly_enabled() {
        // SAFETY: the summarizer switch is read from the process environment,
        // and this test owns the variable for its duration.
        unsafe {
            env::remove_var(LOCAL_SUMMARY_ENV);
            assert!(!local_summary_enabled());

            for off in ["", " ", "0", "false", "no"] {
                env::set_var(LOCAL_SUMMARY_ENV, off);
                assert!(!local_summary_enabled(), "{off:?} must not enable it");
            }

            for on in ["1", "true", "TRUE", " on "] {
                env::set_var(LOCAL_SUMMARY_ENV, on);
                assert!(local_summary_enabled(), "{on:?} must enable it");
            }

            env::remove_var(LOCAL_SUMMARY_ENV);
        }
    }

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
        assert_eq!(
            upsert_managed_block(&format!("before\n{START_MARKER}\nbroken old block"), &block),
            format!("before\n\nbroken old block\n\n{block}\n")
        );
        assert_eq!(
            upsert_managed_block(&format!("broken old block\n{END_MARKER}\nafter"), &block),
            format!("broken old block\n\nafter\n\n{block}\n")
        );
    }

    #[tokio::test]
    async fn upgrades_an_existing_room_to_append_only_session_rules() {
        let root = tempfile::tempdir().unwrap();
        let room_dir = root.path().join(ROOM_DIR);
        fs::create_dir_all(room_dir.join("sessions")).await.unwrap();
        fs::create_dir_all(room_dir.join(LIBRARY_DIR)).await.unwrap();
        fs::write(
            room_dir.join("ROOM.md"),
            "# AI Room: old\nInstruction version: 15\nCreate one session file.",
        )
        .await
        .unwrap();
        fs::write(room_dir.join("sessions/legacy.md"), "legacy body")
            .await
            .unwrap();
        fs::write(
            root.path().join("AGENTS.md"),
            format!("owner before\n{START_MARKER}\nold managed block\n{END_MARKER}\nowner after"),
        )
        .await
        .unwrap();
        let room = AiRoom {
            id: Uuid::nil(),
            name: "existing room".into(),
            description: None,
            local_root: normalized_local_path(root.path()),
            ssh_alias: None,
            remote_root: None,
            instruction_version: 15,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        initialize_local(&room).await.unwrap();

        let instruction = fs::read_to_string(room_dir.join("ROOM.md")).await.unwrap();
        assert!(instruction.contains("Instruction version: 16"));
        assert!(instruction.contains("<random-id>"));
        assert!(instruction.contains("never edit, replace, rename, or delete"));
        let agents = fs::read_to_string(root.path().join("AGENTS.md"))
            .await
            .unwrap();
        assert!(agents.contains("owner before"));
        assert!(agents.contains("owner after"));
        assert!(agents.contains("<random-id>"));
        assert_eq!(agents.matches(START_MARKER).count(), 1);
        assert_eq!(
            fs::read_to_string(room_dir.join("sessions/legacy.md"))
                .await
                .unwrap(),
            "legacy body"
        );
    }

    #[test]
    fn requires_user_visible_progress_reports_every_five_minutes() {
        let room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: None,
            local_root: "C:/work".into(),
            ssh_alias: None,
            remote_root: None,
            instruction_version: INSTRUCTION_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let room_guide = room_instruction(&room);
        let agent_guide = managed_agent_block(&room);

        assert!(room_guide.contains("user-visible progress report"));
        assert!(room_guide.contains("every 5 minutes of wall-clock time"));
        assert!(room_guide.contains("session-file checkpoint does not count"));
        assert!(room_guide.contains("block reporting for 4 minutes or longer"));
        assert!(room_guide.contains("poll it at intervals of at most 60 seconds"));
        assert!(room_guide.contains("library/owner-working-rules.md"));
        assert!(room_guide.contains("one unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<random-id>/`"));
        assert!(room_guide.contains("at least 8 random hexadecimal characters"));
        assert!(room_guide.contains("never edit, replace, rename, or delete an earlier checkpoint"));
        assert!(room_guide.contains(ADVERSARIAL_REVIEW_FILE));
        assert!(room_guide.contains("two independent critics"));
        assert!(agent_guide.contains("visible progress report"));
        assert!(agent_guide.contains("at least every 5 minutes"));
        assert!(agent_guide.contains("Session-file writes do not count"));
        assert!(agent_guide.contains("block reporting for 4 minutes or longer"));
        assert!(agent_guide.contains("library/owner-working-rules.md"));
        assert!(agent_guide.contains("Never rewrite, rename, or delete an existing checkpoint"));
        assert!(agent_guide.contains(ADVERSARIAL_REVIEW_FILE));
        assert!(agent_guide.contains("two-independent-critic"));

        let initial = initial_files(&room);
        assert!(initial.iter().any(|(name, content)| {
            name == OWNER_RULES_FILE && content.contains("프로젝트 소유자의 AI 작업 규칙")
        }));
        assert!(initial.iter().any(|(name, content)| {
            name == ADVERSARIAL_REVIEW_FILE && content.contains("3권 분립형 적대 코드 검토 규약")
        }));
    }

    #[tokio::test]
    async fn indexes_existing_owner_rule_documents() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join(LIBRARY_DIR);
        fs::create_dir_all(&library).await.unwrap();
        for name in [
            "rules-working-agreement.md",
            "result-organization-standard.md",
            "module-ai-cow.md",
            OWNER_RULES_NAME,
        ] {
            fs::write(library.join(name), format!("# {name}"))
                .await
                .unwrap();
        }

        let documents = discover_owner_rule_documents(root.path()).await.unwrap();
        assert_eq!(
            documents,
            vec![
                "result-organization-standard.md".to_string(),
                "rules-working-agreement.md".to_string(),
            ]
        );
        let rendered = owner_working_rules(&documents);
        assert!(rendered.contains("진행 보고와 세션 기록은 별개"));
        assert!(rendered.contains("rules-working-agreement.md"));
        assert!(!rendered.contains("module-ai-cow.md"));
    }

    #[test]
    fn keeps_private_room_records_out_of_git() {
        assert_eq!(
            ensure_room_gitignore_entry("target/\n"),
            "target/\n\n# AI Room records are private, machine-local runtime data\n/.ai-room/\n"
        );
        assert_eq!(
            ensure_room_gitignore_entry("target/\n.ai-room/\n"),
            "target/\n.ai-room/\n"
        );
        assert_eq!(
            ensure_room_gitignore_entry("target/\n/.ai-room\n"),
            "target/\n/.ai-room\n"
        );
    }

    #[tokio::test]
    async fn creates_missing_local_root_and_requires_approval_for_existing_files() {
        let base = tempfile::tempdir().unwrap();
        let missing = base.path().join("new-project");
        assert!(prepare_local_root(&missing, false).await.unwrap());
        assert!(fs::metadata(&missing).await.unwrap().is_dir());

        let existing = base.path().join("existing-project");
        fs::create_dir_all(&existing).await.unwrap();
        fs::write(existing.join("README.md"), "existing")
            .await
            .unwrap();
        let error = prepare_local_root(&existing, false).await.unwrap_err();
        assert!(error.to_string().contains(LOCAL_ROOT_NOT_EMPTY_MARKER));
        assert!(!prepare_local_root(&existing, true).await.unwrap());
    }

    #[tokio::test]
    async fn rejects_folder_with_existing_room_installation() {
        let base = tempfile::tempdir().unwrap();
        let room_dir = base.path().join(ROOM_DIR);
        fs::create_dir_all(&room_dir).await.unwrap();
        fs::write(room_dir.join("room.json"), "{}").await.unwrap();

        let error = prepare_local_root(base.path(), true).await.unwrap_err();
        assert!(error.to_string().contains("another AI Room"));
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
    fn includes_room_description_in_instruction() {
        let mut room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: Some("돼지 데이터 세트 구축 AI를 관리하는 룸".into()),
            local_root: "C:/work".into(),
            ssh_alias: None,
            remote_root: None,
            instruction_version: INSTRUCTION_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(
            room_instruction(&room).contains("Description: 돼지 데이터 세트 구축 AI를 관리하는 룸")
        );
        room.description = None;
        assert!(!room_instruction(&room).contains("Description:"));
    }

    #[test]
    fn keeps_server_room_records_persistent() {
        let room = AiRoom {
            id: Uuid::nil(),
            name: "room".into(),
            description: None,
            local_root: "C:/work".into(),
            ssh_alias: None,
            remote_root: None,
            instruction_version: INSTRUCTION_VERSION,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let guide = room_instruction(&room);
        assert!(guide.contains("Server room records are persistent"));
        assert!(guide.contains("never deletes them automatically"));
        assert!(!guide.contains("removes the temporary server room"));
        assert!(!guide.contains("safe server cleanup"));
    }

    #[test]
    fn recognizes_structured_session_status_and_excludes_generated_index() {
        assert!(is_session_record_path(
            "sessions/20260803-004413-claude-example.md"
        ));
        assert!(is_session_record_path(
            "sessions/20260810-091000-codex-chat-a/000001-start.md"
        ));
        assert!(!is_session_record_path("sessions/INDEX.md"));
        assert!(!is_session_record_path(
            "sessions/chat-a/nested/000001-start.md"
        ));
        assert!(!is_session_record_path(
            "sessions/../000001-start.md"
        ));
        assert_eq!(
            session_status("# Session: example\n\n- Agent: Claude\n- Module: UI\n- Status: 완료\n")
                .as_deref(),
            Some("완료")
        );
        assert_eq!(
            session_status("# 기록\n\n## 상태\n\n- 중단\n").as_deref(),
            Some("중단")
        );
        assert!(status_is_inactive("완료"));
        assert!(status_is_inactive("보류"));
        assert!(!status_is_inactive("진행중"));
        assert!(!status_is_inactive(
            "**진행 중** — 기록 통합 완료, 환경 구축은 미착수"
        ));
        assert!(status_is_inactive("**완료** — 검증까지 끝남"));
    }

    #[test]
    fn groups_append_only_checkpoints_by_conversation_and_uses_latest_status() {
        let local = BTreeMap::from([
            (
                "sessions/20260810-091000-codex-chat-a/000001-start.md".into(),
                "# Session: example\n- Status: 진행중\n\nstarted".into(),
            ),
            (
                "sessions/20260810-091000-codex-chat-a/000002-finish.md".into(),
                format!(
                    "# Session: example\n- Status: 완료\n\nfinished\n{SESSION_COMPLETE_MARKER}"
                ),
            ),
            (
                "sessions/20260809-120000-claude-legacy.md".into(),
                "legacy".into(),
            ),
        ]);

        let (records, conflicts) = aggregate_session_records(&local, &BTreeMap::new());

        assert!(conflicts.is_empty());
        assert_eq!(records.len(), 2);
        let conversation = records
            .iter()
            .find(|record| record.filename.ends_with("codex-chat-a"))
            .unwrap();
        assert_eq!(
            conversation
                .content
                .matches("<!-- AI Room checkpoint:")
                .count(),
            2
        );
        assert_eq!(session_status(&conversation.content).as_deref(), Some("완료"));
        assert!(session_is_complete(&conversation.content));
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
        assert_eq!(rendered.matches("- ⏳ 진행: 벤치마크 실행").count(), 1);
        assert!(!rendered.contains("- ⏳ 진행: 결과 폴더 정리"));
        assert!(rendered.contains("- ✅ 완료: 결과 폴더 정리"));
        assert!(rendered.contains("- ✅ 완료: 표 / 통합 완료"));
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
        assert!(rendered.contains("- ⏳ 진행: pair-identity 분기(exp34+)."));
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
        assert!(rendered.contains("- ✅ 완료: Keep this result"));
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
    async fn preserves_a_diverged_legacy_session_in_place() {
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

        assert!(copied.is_empty());
        assert_eq!(conflicts, vec!["sessions/active.md"]);
        assert_eq!(
            fs::read_to_string(room_dir.join("sessions/active.md"))
                .await
                .unwrap(),
            "checkpoint one"
        );
    }

    #[tokio::test]
    async fn reads_nested_conversation_checkpoints_without_flattening_paths() {
        let root = tempfile::tempdir().unwrap();
        let conversation = root
            .path()
            .join(ROOM_DIR)
            .join("sessions/20260810-091000-codex-chat-a");
        fs::create_dir_all(&conversation).await.unwrap();
        fs::write(conversation.join("000001-start.md"), "first")
            .await
            .unwrap();
        fs::write(conversation.join("000002-checkpoint.md"), "second")
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

        assert_eq!(
            files
                .files
                .get("sessions/20260810-091000-codex-chat-a/000001-start.md")
                .map(String::as_str),
            Some("first")
        );
        assert_eq!(
            files
                .files
                .get("sessions/20260810-091000-codex-chat-a/000002-checkpoint.md")
                .map(String::as_str),
            Some("second")
        );
    }

    #[tokio::test]
    async fn never_overwrites_a_diverged_append_only_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let checkpoint = root
            .path()
            .join(ROOM_DIR)
            .join("sessions/20260810-091000-codex-chat-a/000001-start.md");
        fs::create_dir_all(checkpoint.parent().unwrap())
            .await
            .unwrap();
        fs::write(&checkpoint, "local immutable body").await.unwrap();
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
        let relative = "sessions/20260810-091000-codex-chat-a/000001-start.md";
        let remote = EndpointFiles {
            available: true,
            files: BTreeMap::from([(relative.into(), "remote replacement".into())]),
            ..Default::default()
        };

        let (copied, conflicts) = sync_remote_checkpoints(&room, &mut local, &remote)
            .await
            .unwrap();

        assert!(copied.is_empty());
        assert_eq!(conflicts, vec![relative]);
        assert_eq!(fs::read_to_string(checkpoint).await.unwrap(), "local immutable body");
    }

    #[tokio::test]
    async fn create_new_session_record_never_replaces_a_racing_writer() {
        let root = tempfile::tempdir().unwrap();
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
        let relative = "sessions/20260810-091000-codex-chat-a/000001-start.md";

        assert!(
            create_local_session_record(&room, relative, "racing local body")
                .await
                .unwrap()
        );
        assert!(
            !create_local_session_record(&room, relative, "remote body")
                .await
                .unwrap()
        );
        assert_eq!(
            fs::read_to_string(safe_room_path(&room, relative).unwrap())
                .await
                .unwrap(),
            "racing local body"
        );
    }

    #[tokio::test]
    async fn accepts_diverged_server_records_and_preserves_local_history() {
        let root = tempfile::tempdir().unwrap();
        let room_dir = root.path().join(ROOM_DIR);
        fs::create_dir_all(room_dir.join("sessions")).await.unwrap();
        fs::create_dir_all(room_dir.join(LIBRARY_DIR))
            .await
            .unwrap();
        fs::write(room_dir.join("sessions/active.md"), "local session edit")
            .await
            .unwrap();
        fs::write(
            room_dir.join(LIBRARY_DIR).join("guide.md"),
            "local guide edit",
        )
        .await
        .unwrap();
        fs::write(room_dir.join("context.md"), "local context edit")
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
            files: BTreeMap::from([
                ("sessions/active.md".into(), "server session edit".into()),
                ("library/guide.md".into(), "server guide edit".into()),
                ("context.md".into(), "server context edit".into()),
            ]),
            ..Default::default()
        };

        let (copied, conflicts) = sync_remote_checkpoints(&room, &mut local, &remote)
            .await
            .unwrap();

        assert_eq!(conflicts, vec!["sessions/active.md"]);
        assert_eq!(copied.len(), 1);
        assert_eq!(
            fs::read_to_string(room_dir.join("sessions/active.md"))
                .await
                .unwrap(),
            "local session edit"
        );
        assert_eq!(
            fs::read_to_string(room_dir.join(LIBRARY_DIR).join("guide.md"))
                .await
                .unwrap(),
            "server guide edit"
        );
        // The owner-authored context keeps the local edit; the server copy is
        // updated instead of the other way around.
        assert_eq!(
            fs::read_to_string(room_dir.join("context.md"))
                .await
                .unwrap(),
            "local context edit"
        );
        assert_eq!(
            fs::read_to_string(
                room_dir
                    .join(LOCAL_HISTORY_DIR)
                    .join(content_hash("local session edit"))
                    .join("sessions/active.md")
            )
            .await
            .unwrap(),
            "local session edit"
        );
        assert_eq!(
            fs::read_to_string(
                room_dir
                    .join(LOCAL_HISTORY_DIR)
                    .join(content_hash("local guide edit"))
                    .join("library/guide.md")
            )
            .await
            .unwrap(),
            "local guide edit"
        );
    }
}
