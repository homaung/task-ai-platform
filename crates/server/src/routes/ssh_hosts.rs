use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use axum::{
    Router,
    extract::{Query, State, ws::Message},
    response::{IntoResponse, Json as ResponseJson},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::{Instant, MissedTickBehavior, timeout},
};
use ts_rs::TS;
use utils::{command_ext::NoWindowExt, response::ApiResponse};
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::signed_ws::{MaybeSignedWebSocket, SignedWsUpgrade},
};

const SSH_TIMEOUT: Duration = Duration::from_secs(15);
const PROGRESS_REPORT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const PROGRESS_REMINDER_POLL: Duration = Duration::from_secs(1);
const PROGRESS_REMINDER_INPUT_IDLE: Duration = Duration::from_secs(2);
const SAFE_SET_ENV: &str = "SetEnv=TASK_AI_PLATFORM=1";
const CLEAR_SEND_ENV: &str = "SendEnv=-*";
const PROGRESS_REMINDER: &str = "지금 하던 작업을 취소하지 말고, 먼저 사용자에게 현재까지 완료한 일, 지금 하는 일, 막힌 점, 다음 할 일을 짧게 중간 보고한 뒤 계속 작업하세요. 세션 파일 기록만 하지 말고 사용자 대화에 직접 보고하세요.\r";

#[derive(Debug, Clone, Serialize, TS)]
pub struct SshHostSummary {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct SshHostsResponse {
    pub ssh_available: bool,
    pub config_path: Option<String>,
    pub hosts: Vec<SshHostSummary>,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct InspectSshHostRequest {
    pub alias: String,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct SshRemoteTool {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct SshConnectionInfo {
    pub alias: String,
    pub home_dir: String,
    pub shell: String,
    pub tools: Vec<SshRemoteTool>,
    pub repositories: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SshLaunchTool {
    Shell,
    Claude,
    Codex,
}

#[derive(Debug, Deserialize)]
pub struct SshTerminalQuery {
    pub alias: String,
    pub path: String,
    #[serde(default = "default_launch_tool")]
    pub tool: SshLaunchTool,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalCommand {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalMessage {
    Output { data: String },
    Error { message: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentCommand {
    Start { prompt: String },
    Cancel,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentMessage {
    Started,
    Stdout { line: String },
    Stderr { line: String },
    Finished { success: bool, code: Option<i32> },
    Cancelled,
    Error { message: String },
}

fn default_launch_tool() -> SshLaunchTool {
    SshLaunchTool::Shell
}

fn default_cols() -> u16 {
    100
}

fn default_rows() -> u16 {
    30
}

fn ssh_config_path() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".ssh").join("config"))
}

fn is_safe_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 255
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn parse_host_aliases(content: &str) -> Vec<String> {
    let mut aliases = BTreeSet::new();

    for raw_line in content.lines() {
        let line = raw_line
            .trim_start_matches('\u{feff}')
            .split('#')
            .next()
            .unwrap_or("")
            .trim();
        let Some((keyword, value)) = line
            .split_once(char::is_whitespace)
            .map(|(keyword, value)| (keyword.trim(), value.trim()))
        else {
            continue;
        };

        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }

        for alias in value.split_whitespace() {
            if is_safe_alias(alias) {
                aliases.insert(alias.to_string());
            }
        }
    }

    aliases.into_iter().collect()
}

pub(crate) async fn run_ssh(
    args: &[String],
    command_timeout: Duration,
) -> Result<std::process::Output, ApiError> {
    run_ssh_process(args, None, command_timeout).await
}

pub(crate) async fn run_ssh_with_stdin(
    args: &[String],
    input: &[u8],
    command_timeout: Duration,
) -> Result<std::process::Output, ApiError> {
    run_ssh_process(args, Some(input.to_vec()), command_timeout).await
}

async fn run_ssh_process(
    args: &[String],
    input: Option<Vec<u8>>,
    command_timeout: Duration,
) -> Result<std::process::Output, ApiError> {
    let mut command = Command::new("ssh");
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_window();
    let mut child = command
        .spawn()
        .map_err(|error| ApiError::BadRequest(format!("Unable to run ssh: {error}")))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::BadRequest("Unable to open SSH output".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ApiError::BadRequest("Unable to open SSH error output".into()))?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stdin_task = input.map(|bytes| {
        let mut stdin = child.stdin.take().expect("piped SSH stdin");
        tokio::spawn(async move {
            stdin.write_all(&bytes).await?;
            stdin.shutdown().await
        })
    });

    let status = match timeout(command_timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(ApiError::BadRequest(format!("Unable to run ssh: {error}")));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            if let Some(task) = stdin_task {
                task.abort();
            }
            return Err(ApiError::BadRequest("SSH command timed out".into()));
        }
    };
    if let Some(task) = stdin_task {
        task.await
            .map_err(|error| ApiError::BadRequest(format!("SSH input task failed: {error}")))?
            .map_err(|error| ApiError::BadRequest(format!("Unable to write SSH input: {error}")))?;
    }
    let stdout = stdout_task
        .await
        .map_err(|error| ApiError::BadRequest(format!("SSH output task failed: {error}")))?
        .map_err(|error| ApiError::BadRequest(format!("Unable to read SSH output: {error}")))?;
    let stderr = stderr_task
        .await
        .map_err(|error| ApiError::BadRequest(format!("SSH error task failed: {error}")))?
        .map_err(|error| {
            ApiError::BadRequest(format!("Unable to read SSH error output: {error}"))
        })?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn parse_resolved_host(alias: String, output: &str) -> SshHostSummary {
    let mut hostname = alias.clone();
    let mut user = None;
    let mut port = None;
    let mut identity_files = Vec::new();

    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "hostname" => hostname = value.to_string(),
            "user" => user = Some(value.to_string()),
            "port" => port = value.parse().ok(),
            "identityfile" if !identity_files.iter().any(|item| item == value) => {
                identity_files.push(value.to_string());
            }
            _ => {}
        }
    }

    SshHostSummary {
        alias,
        hostname,
        user,
        port,
        identity_files,
    }
}

async fn discovered_hosts() -> Result<SshHostsResponse, ApiError> {
    let ssh_available = Command::new("ssh")
        .arg("-V")
        .no_window()
        .output()
        .await
        .is_ok();
    let config_path = ssh_config_path();
    let Some(path) = config_path.as_ref() else {
        return Ok(SshHostsResponse {
            ssh_available,
            config_path: None,
            hosts: Vec::new(),
        });
    };

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(ApiError::BadRequest(format!(
                "Unable to read {}: {error}",
                path.display()
            )));
        }
    };

    let hosts = join_all(
        parse_host_aliases(&content)
            .into_iter()
            .map(|alias| async move {
                let args = vec![
                    "-G".to_string(),
                    "-o".to_string(),
                    SAFE_SET_ENV.to_string(),
                    "-o".to_string(),
                    CLEAR_SEND_ENV.to_string(),
                    alias.clone(),
                ];
                match run_ssh(&args, Duration::from_secs(3)).await {
                    Ok(output) if output.status.success() => {
                        parse_resolved_host(alias, &String::from_utf8_lossy(&output.stdout))
                    }
                    _ => SshHostSummary {
                        hostname: alias.clone(),
                        alias,
                        user: None,
                        port: None,
                        identity_files: Vec::new(),
                    },
                }
            }),
    )
    .await;

    Ok(SshHostsResponse {
        ssh_available,
        config_path: Some(path.to_string_lossy().to_string()),
        hosts,
    })
}

pub(crate) async fn require_registered_alias(alias: &str) -> Result<(), ApiError> {
    if !is_safe_alias(alias) {
        return Err(ApiError::BadRequest("Invalid SSH host alias".to_string()));
    }

    let path = ssh_config_path()
        .ok_or_else(|| ApiError::BadRequest("SSH config path is unavailable".to_string()))?;
    let content = fs::read_to_string(&path).map_err(|error| {
        ApiError::BadRequest(format!("Unable to read {}: {error}", path.display()))
    })?;
    if parse_host_aliases(&content)
        .iter()
        .any(|host| host == alias)
    {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "SSH host is not registered in ~/.ssh/config".to_string(),
        ))
    }
}

pub(crate) fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn remote_shell_command(script: &str) -> String {
    format!("sh -lc {}", posix_quote(script))
}

fn inspect_script() -> &'static str {
    r#"printf 'home\t%s\n' "$HOME"
printf 'shell\t%s\n' "${SHELL:-/bin/sh}"
for tool in git claude codex; do
  tool_path=$(command -v "$tool" 2>/dev/null || true)
  if [ -n "$tool_path" ]; then printf 'tool\t%s\t%s\n' "$tool" "$tool_path"; fi
done
{
for root in "$HOME/works" "$HOME/workspace" "$HOME/workspaces" "$HOME/projects" "$HOME/code" /workspace /workspaces /srv /opt; do
  [ -d "$root" ] || continue
  find "$root" -mindepth 1 -maxdepth 1 -type d -print 2>/dev/null
done
for root in "$HOME/works" "$HOME/workspace" "$HOME/workspaces" "$HOME/projects" "$HOME/code" /workspace /workspaces /srv /opt; do
  [ -d "$root" ] || continue
  find "$root" -maxdepth 3 -type d -name .git -print 2>/dev/null
done
} | sed 's#/.git$##' | awk '$0 !~ /\/\.([^/]+)(\/|$)/ && $0 !~ /\/(node_modules|target|vendor)(\/|$)/ && !seen[$0]++' | head -80 | while IFS= read -r repo; do
  printf 'repo\t%s\n' "$repo"
done"#
}

fn parse_connection_info(alias: String, stdout: &str) -> SshConnectionInfo {
    let mut home_dir = String::new();
    let mut shell = "/bin/sh".to_string();
    let mut tools = Vec::new();
    let mut repositories = Vec::new();

    for line in stdout.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["home", value] => home_dir = (*value).to_string(),
            ["shell", value] => shell = (*value).to_string(),
            ["tool", name, path] => tools.push(SshRemoteTool {
                name: (*name).to_string(),
                path: (*path).to_string(),
            }),
            ["repo", path] => repositories.push((*path).to_string()),
            _ => {}
        }
    }

    SshConnectionInfo {
        alias,
        home_dir,
        shell,
        tools,
        repositories,
    }
}

pub async fn list_ssh_hosts() -> Result<ResponseJson<ApiResponse<SshHostsResponse>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        discovered_hosts().await?,
    )))
}

pub async fn inspect_ssh_host(
    State(_deployment): State<DeploymentImpl>,
    axum::Json(request): axum::Json<InspectSshHostRequest>,
) -> Result<ResponseJson<ApiResponse<SshConnectionInfo>>, ApiError> {
    require_registered_alias(&request.alias).await?;

    let args = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=8".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
        "-o".to_string(),
        SAFE_SET_ENV.to_string(),
        "-o".to_string(),
        CLEAR_SEND_ENV.to_string(),
        request.alias.clone(),
        remote_shell_command(inspect_script()),
    ];
    let output = run_ssh(&args, SSH_TIMEOUT).await?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::BadRequest(if error.is_empty() {
            format!("Unable to connect to {}", request.alias)
        } else {
            error
        }));
    }

    let info = parse_connection_info(request.alias, &String::from_utf8_lossy(&output.stdout));
    Ok(ResponseJson(ApiResponse::success(info)))
}

fn terminal_script(path: &str, tool: SshLaunchTool, session_dir: Option<&str>) -> String {
    let quoted_path = posix_quote(path);
    let executable = match tool {
        SshLaunchTool::Shell => "exec \"${SHELL:-/bin/sh}\" -l",
        SshLaunchTool::Claude => "exec claude",
        SshLaunchTool::Codex => "exec codex",
    };
    let session_setup = session_dir.map_or_else(String::new, |session_dir| {
        let session_leaf = session_dir.rsplit('/').next().unwrap_or(session_dir);
        let agent = match tool {
            SshLaunchTool::Claude => "Claude",
            SshLaunchTool::Codex => "Codex",
            SshLaunchTool::Shell => "Shell",
        };
        let started = Utc::now().format("%Y-%m-%d %H:%M UTC");
        let body = format!(
            "# Session: 새 대화 시작\n- Agent: {agent}\n- Module: project\n- Status: 진행중\n- Started: {started}\n\n## Goal\n\n이 대화의 작업 목표와 첫 결과를 다음 체크포인트에 기록한다.\n"
        );
        format!(
            "room_root=$PWD; while [ \"$room_root\" != / ] && [ ! -d \"$room_root/.ai-room/sessions\" ]; do parent=$(dirname -- \"$room_root\"); [ \"$parent\" = \"$room_root\" ] && break; room_root=$parent; done; if [ -d \"$room_root/.ai-room/sessions\" ]; then session_dir=\"$room_root/.ai-room/sessions/\"{}; mkdir -- \"$session_dir\" || exit 1; printf '%s' {} | base64 -d > \"$session_dir/000001-start.md\" || exit 1; export TASK_AI_SESSION_DIR=\"$session_dir\"; fi; ",
            posix_quote(session_leaf),
            posix_quote(&BASE64.encode(body))
        )
    });
    format!("cd -- {quoted_path} || exit 1; {session_setup}{executable}")
}

fn new_terminal_session_dir(tool: SshLaunchTool) -> Option<String> {
    let agent = match tool {
        SshLaunchTool::Shell => return None,
        SshLaunchTool::Claude => "claude",
        SshLaunchTool::Codex => "codex",
    };
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let random = &Uuid::new_v4().simple().to_string()[..12];
    Some(format!(".ai-room/sessions/{timestamp}-{agent}-{random}"))
}

fn agent_script(path: &str, tool: SshLaunchTool, prompt: &str) -> Result<String, ApiError> {
    if prompt.trim().is_empty() || prompt.len() > 24_000 {
        return Err(ApiError::BadRequest(
            "Prompt must be between 1 and 24000 bytes".to_string(),
        ));
    }

    let command = match tool {
        SshLaunchTool::Claude => format!(
            "exec claude -p --verbose --output-format=stream-json --include-partial-messages --permission-mode auto {}",
            posix_quote(prompt)
        ),
        SshLaunchTool::Codex => {
            format!("exec codex exec --json --full-auto {}", posix_quote(prompt))
        }
        SshLaunchTool::Shell => {
            return Err(ApiError::BadRequest(
                "Shell is not a chat agent".to_string(),
            ));
        }
    };

    Ok(format!("cd -- {} || exit 1; {command}", posix_quote(path)))
}

async fn send_agent_message(socket: &mut MaybeSignedWebSocket, message: &AgentMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn receive_agent_start(socket: &mut MaybeSignedWebSocket) -> Result<String, &'static str> {
    loop {
        match socket.recv().await {
            Ok(Some(Message::Text(text))) => {
                let command = serde_json::from_str::<AgentCommand>(text.as_str())
                    .map_err(|_| "Invalid agent command")?;
                match command {
                    AgentCommand::Start { prompt } => return Ok(prompt),
                    AgentCommand::Cancel => return Err("Agent run cancelled"),
                }
            }
            Ok(Some(Message::Close(_))) | Ok(None) => return Err("Connection closed"),
            Ok(Some(_)) => {}
            Err(_) => return Err("Connection closed"),
        }
    }
}

pub async fn ssh_agent_ws(
    ws: SignedWsUpgrade,
    State(_deployment): State<DeploymentImpl>,
    Query(query): Query<SshTerminalQuery>,
) -> Result<impl IntoResponse, ApiError> {
    require_registered_alias(&query.alias).await?;
    if query.path.is_empty() || query.path.len() > 4096 || query.path.contains(['\r', '\n', '\0']) {
        return Err(ApiError::BadRequest("Invalid remote path".to_string()));
    }
    if matches!(query.tool, SshLaunchTool::Shell) {
        return Err(ApiError::BadRequest(
            "Select Claude or Codex for agent chat".to_string(),
        ));
    }

    Ok(ws.on_upgrade(move |socket| handle_ssh_agent(socket, query)))
}

async fn handle_ssh_agent(mut socket: MaybeSignedWebSocket, query: SshTerminalQuery) {
    let prompt = match receive_agent_start(&mut socket).await {
        Ok(prompt) => prompt,
        Err(message) => {
            let _ = send_agent_message(
                &mut socket,
                &AgentMessage::Error {
                    message: message.to_string(),
                },
            )
            .await;
            return;
        }
    };
    let script = match agent_script(&query.path, query.tool, &prompt) {
        Ok(script) => script,
        Err(error) => {
            let _ = send_agent_message(
                &mut socket,
                &AgentMessage::Error {
                    message: error.to_string(),
                },
            )
            .await;
            return;
        }
    };

    let mut child = match Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "LogLevel=ERROR",
            "-o",
            SAFE_SET_ENV,
            "-o",
            CLEAR_SEND_ENV,
            &query.alias,
            &remote_shell_command(&script),
        ])
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_window()
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = send_agent_message(
                &mut socket,
                &AgentMessage::Error {
                    message: format!("Unable to start SSH agent: {error}"),
                },
            )
            .await;
            return;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        let _ = send_agent_message(
            &mut socket,
            &AgentMessage::Error {
                message: "SSH agent stdout is unavailable".to_string(),
            },
        )
        .await;
        return;
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = send_agent_message(
            &mut socket,
            &AgentMessage::Error {
                message: "SSH agent stderr is unavailable".to_string(),
            },
        )
        .await;
        return;
    };

    if !send_agent_message(&mut socket, &AgentMessage::Started).await {
        let _ = child.kill().await;
        return;
    }

    let mut stdout = BufReader::new(stdout).lines();
    let mut stderr = BufReader::new(stderr).lines();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut cancelled = false;

    while stdout_open || stderr_open {
        tokio::select! {
            line = stdout.next_line(), if stdout_open => {
                match line {
                    Ok(Some(line)) => {
                        if !send_agent_message(&mut socket, &AgentMessage::Stdout { line }).await {
                            let _ = child.kill().await;
                            return;
                        }
                    }
                    Ok(None) | Err(_) => stdout_open = false,
                }
            }
            line = stderr.next_line(), if stderr_open => {
                match line {
                    Ok(Some(line)) => {
                        if !send_agent_message(&mut socket, &AgentMessage::Stderr { line }).await {
                            let _ = child.kill().await;
                            return;
                        }
                    }
                    Ok(None) | Err(_) => stderr_open = false,
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Ok(Some(Message::Text(text))) => {
                        if matches!(serde_json::from_str::<AgentCommand>(text.as_str()), Ok(AgentCommand::Cancel)) {
                            cancelled = true;
                            let _ = child.kill().await;
                            break;
                        }
                    }
                    Ok(Some(Message::Close(_))) | Ok(None) | Err(_) => {
                        let _ = child.kill().await;
                        return;
                    }
                    Ok(Some(_)) => {}
                }
            }
        }
    }

    let status = child.wait().await.ok();
    let message = if cancelled {
        AgentMessage::Cancelled
    } else {
        AgentMessage::Finished {
            success: status.as_ref().is_some_and(|status| status.success()),
            code: status.and_then(|status| status.code()),
        }
    };
    let _ = send_agent_message(&mut socket, &message).await;
}

pub async fn ssh_terminal_ws(
    ws: SignedWsUpgrade,
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<SshTerminalQuery>,
) -> Result<impl IntoResponse, ApiError> {
    require_registered_alias(&query.alias).await?;
    if query.path.is_empty() || query.path.len() > 4096 || query.path.contains(['\r', '\n', '\0']) {
        return Err(ApiError::BadRequest("Invalid remote path".to_string()));
    }

    Ok(ws.on_upgrade(move |socket| handle_ssh_terminal(socket, deployment, query)))
}

async fn handle_ssh_terminal(
    mut socket: MaybeSignedWebSocket,
    deployment: DeploymentImpl,
    query: SshTerminalQuery,
) {
    let progress_watch_enabled = !matches!(query.tool, SshLaunchTool::Shell);
    let args = vec![
        "-tt".to_string(),
        "-o".to_string(),
        SAFE_SET_ENV.to_string(),
        "-o".to_string(),
        CLEAR_SEND_ENV.to_string(),
        query.alias,
        remote_shell_command(&terminal_script(
            &query.path,
            query.tool,
            new_terminal_session_dir(query.tool).as_deref(),
        )),
    ];
    let working_dir = env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let (session_id, mut output_rx) = match deployment
        .pty()
        .create_process_session(
            PathBuf::from("ssh"),
            args,
            working_dir,
            query.cols,
            query.rows,
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            let _ = send_error(&mut socket, &error.to_string()).await;
            return;
        }
    };

    let pty_service = deployment.pty().clone();
    let mut reminder_timer = tokio::time::interval(PROGRESS_REMINDER_POLL);
    reminder_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut next_report_deadline: Option<Instant> = None;
    let mut last_user_input = Instant::now();
    loop {
        tokio::select! {
            maybe_output = output_rx.recv() => {
                let Some(data) = maybe_output else { break; };
                let message = TerminalMessage::Output { data: BASE64.encode(&data) };
                if let Ok(json) = serde_json::to_string(&message)
                    && socket.send(Message::Text(json.into())).await.is_err()
                {
                    break;
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Ok(Some(Message::Text(text))) => {
                        if let Ok(command) = serde_json::from_str::<TerminalCommand>(text.as_str()) {
                            match command {
                                TerminalCommand::Input { data } => {
                                    if let Ok(bytes) = BASE64.decode(data) {
                                        last_user_input = Instant::now();
                                        let first_task_submission = progress_watch_enabled
                                            && next_report_deadline.is_none()
                                            && bytes
                                                .iter()
                                                .any(|byte| matches!(*byte, b'\r' | b'\n'));
                                        if first_task_submission {
                                            next_report_deadline = Some(
                                                Instant::now() + PROGRESS_REPORT_INTERVAL
                                            );
                                        }
                                        let _ = pty_service.write(session_id, &bytes).await;
                                    }
                                }
                                TerminalCommand::Resize { cols, rows } => {
                                    let _ = pty_service.resize(session_id, cols, rows).await;
                                }
                            }
                        }
                    }
                    Ok(Some(Message::Close(_))) | Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => break,
                }
            }
            _ = reminder_timer.tick(), if next_report_deadline.is_some() => {
                let now = Instant::now();
                if next_report_deadline.is_some_and(|deadline| now >= deadline)
                    && now.duration_since(last_user_input) >= PROGRESS_REMINDER_INPUT_IDLE
                {
                    let notice = "\r\n\x1b[33m[Task AI Platform] 5분이 지나 AI에게 중간 보고를 요청했습니다.\x1b[0m\r\n";
                    let message = TerminalMessage::Output {
                        data: BASE64.encode(notice.as_bytes()),
                    };
                    if let Ok(json) = serde_json::to_string(&message)
                        && socket.send(Message::Text(json.into())).await.is_err()
                    {
                        break;
                    }
                    let _ = pty_service
                        .write(session_id, PROGRESS_REMINDER.as_bytes())
                        .await;
                    next_report_deadline = Some(now + PROGRESS_REPORT_INTERVAL);
                }
            }
        }
    }

    let _ = deployment.pty().close_session(session_id).await;
}

async fn send_error(socket: &mut MaybeSignedWebSocket, message: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string(&TerminalMessage::Error {
        message: message.to_string(),
    })?;
    socket.send(Message::Text(json.into())).await?;
    socket.close().await?;
    Ok(())
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/ssh/hosts", get(list_ssh_hosts))
        .route("/ssh/inspect", post(inspect_ssh_host))
        .route("/ssh/agent/ws", get(ssh_agent_ws))
        .route("/ssh/terminal/ws", get(ssh_terminal_ws))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        PROGRESS_REMINDER, PROGRESS_REPORT_INTERVAL, SshLaunchTool, agent_script, is_safe_alias,
        new_terminal_session_dir, parse_host_aliases, posix_quote, terminal_script,
    };

    #[test]
    fn configures_five_minute_runtime_progress_reminders() {
        assert_eq!(PROGRESS_REPORT_INTERVAL, Duration::from_secs(300));
        assert!(PROGRESS_REMINDER.contains("중간 보고"));
        assert!(PROGRESS_REMINDER.contains("사용자 대화에 직접 보고"));
        assert!(PROGRESS_REMINDER.ends_with('\r'));
    }

    #[test]
    fn allocates_a_unique_immutable_session_directory_for_each_agent_terminal() {
        let first = new_terminal_session_dir(SshLaunchTool::Claude).unwrap();
        let second = new_terminal_session_dir(SshLaunchTool::Claude).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(".ai-room/sessions/"));
        assert!(first.contains("-claude-"));
        assert!(new_terminal_session_dir(SshLaunchTool::Shell).is_none());

        let script = terminal_script("/work/project", SshLaunchTool::Claude, Some(&first));
        assert!(script.contains("while [ \"$room_root\" != / ]"));
        assert!(script.contains("[ -d \"$room_root/.ai-room/sessions\" ]"));
        assert!(script.contains("mkdir -- \"$session_dir\""));
        assert!(script.contains("000001-start.md"));
        assert!(script.contains("TASK_AI_SESSION_DIR"));
        assert!(script.contains("exec claude"));

        let plain = terminal_script("/work/project", SshLaunchTool::Claude, None);
        assert!(!plain.contains("TASK_AI_SESSION_DIR"));
        assert!(plain.contains("exec claude"));
    }

    #[test]
    fn parses_concrete_hosts_and_ignores_patterns() {
        let config = r#"
Host server_250 gpu-box
  HostName 10.0.0.250
Host *
  ServerAliveInterval 30
Host !blocked internal-?.example.com
"#;

        assert_eq!(
            parse_host_aliases(config),
            vec!["gpu-box".to_string(), "server_250".to_string()]
        );
    }

    #[test]
    fn parses_first_host_after_utf8_bom() {
        let config = "\u{feff}Host intflow-gpu-node\n  HostName example.internal\n\nHost spark23\n";
        assert_eq!(
            parse_host_aliases(config),
            vec!["intflow-gpu-node".to_string(), "spark23".to_string()]
        );
    }

    #[test]
    fn validates_aliases_and_quotes_remote_paths() {
        assert!(is_safe_alias("server_250.example"));
        assert!(!is_safe_alias("user@server"));
        assert!(!is_safe_alias("server; shutdown"));
        assert_eq!(
            posix_quote("/work/user's repo"),
            "'/work/user'\"'\"'s repo'"
        );
    }

    #[test]
    fn builds_quoted_agent_commands() {
        let command = agent_script(
            "/work/user's repo",
            SshLaunchTool::Claude,
            "fix the user's bug",
        )
        .unwrap();
        assert!(command.contains("cd -- '/work/user'\"'\"'s repo'"));
        assert!(command.contains("'fix the user'\"'\"'s bug'"));
    }
}
