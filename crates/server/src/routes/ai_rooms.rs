use std::{
    collections::BTreeMap,
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
const INSTRUCTION_VERSION: i64 = 1;
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;
const CLEAR_SEND_ENV: &str = "SendEnv=-*";
const START_MARKER: &str = "<!-- task-ai-room:start -->";
const END_MARKER: &str = "<!-- task-ai-room:end -->";
const AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(15);
static SYNC_LOCK: Mutex<()> = Mutex::const_new(());

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

        for document in ["context.md", "decisions.md", "tasks.md"] {
            if let (Some(left), Some(right)) =
                (local.files.get(document), remote.files.get(document))
                && left != right
            {
                conflicts.push(document.to_string());
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
    tokio::spawn(async move {
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
                let has_remote_session = remote.available
                    && remote
                        .files
                        .keys()
                        .any(|name| name.starts_with("sessions/"));
                if !has_remote_session {
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
        "# AI Room: {name}\n\nRoom ID: `{id}`\nInstruction version: {version}\n\n## Required session workflow\n\n1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, and the newest files in `.ai-room/sessions/`.\n2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.\n3. Record the goal, assumptions, important commands, files changed, verification results, decisions, blockers, and concrete next steps. Update it during the session, not only at the end.\n4. Update `tasks.md` when task status changes. Update `context.md` only for durable project facts. Append architectural decisions to `decisions.md`; do not rewrite history.\n5. Never store secrets, tokens, private keys, or raw credentials in room files.\n6. Before ending, make the session file sufficient for another Claude or Codex session to continue without relying on chat history.\n\n## Server privacy\n\nWhen this room is prepared on its SSH server, all `.ai-room` files there are temporary. The Task AI Platform copies completed session files to the local root and removes the server-side room files after a conflict-free sync. Do not assume earlier session files remain on the server.\n\n## Room endpoints\n\n- Local root: `{local}`\n- Remote root: `{remote}`\n\nThe Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.\n",
        name = room.name,
        id = room.id,
        version = INSTRUCTION_VERSION,
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
            "# Tasks\n\n- [ ] Describe the next concrete task.\n".into(),
        ),
    ]
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
}
