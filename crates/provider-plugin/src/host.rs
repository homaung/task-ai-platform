use std::{collections::BTreeSet, process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};
use ts_rs::TS;

use crate::DiscoveredProviderPlugin;

#[derive(Debug, Clone)]
pub struct PluginHostPolicy {
    pub approved_permissions: BTreeSet<String>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for PluginHostPolicy {
    fn default() -> Self {
        Self {
            approved_permissions: BTreeSet::new(),
            timeout: Duration::from_secs(30),
            max_response_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProviderPluginRequest {
    pub id: String,
    pub method: String,
    #[ts(type = "unknown")]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProviderProtocolError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProviderPluginResponse {
    pub id: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<ProviderProtocolError>,
}

#[derive(Debug, Error)]
pub enum PluginHostError {
    #[error("plugin requires unapproved permissions: {0:?}")]
    PermissionDenied(Vec<String>),
    #[error("failed to start isolated plugin process: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("plugin process did not expose stdin")]
    MissingStdin,
    #[error("plugin process did not expose stdout")]
    MissingStdout,
    #[error("plugin call timed out")]
    Timeout,
    #[error("plugin response exceeded the configured size limit")]
    ResponseTooLarge,
    #[error("plugin closed without returning a response")]
    EmptyResponse,
    #[error("invalid plugin response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("plugin response id does not match the request id")]
    ResponseIdMismatch,
    #[error("plugin returned an error: {code}: {message}")]
    Plugin { code: String, message: String },
}

pub struct PluginHost;

impl PluginHost {
    pub async fn call(
        plugin: &DiscoveredProviderPlugin,
        request: ProviderPluginRequest,
        policy: &PluginHostPolicy,
    ) -> Result<Value, PluginHostError> {
        let required: BTreeSet<_> = plugin.manifest.permissions.iter().cloned().collect();
        let missing: Vec<_> = required
            .difference(&policy.approved_permissions)
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(PluginHostError::PermissionDenied(missing));
        }

        let mut command = command_for_entrypoint(&plugin.entrypoint_path);
        let mut child = command
            .arg("--provider-plugin-rpc")
            .current_dir(&plugin.root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let mut stdin = child.stdin.take().ok_or(PluginHostError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(PluginHostError::MissingStdout)?;
        let payload = serde_json::to_vec(&request)?;
        stdin.write_all(&payload).await?;
        stdin.write_all(b"\n").await?;
        stdin.shutdown().await?;

        let read_response = async {
            let mut reader = BufReader::new(stdout);
            let mut response_line = Vec::new();
            let bytes = reader.read_until(b'\n', &mut response_line).await?;
            if bytes == 0 {
                return Err(PluginHostError::EmptyResponse);
            }
            if response_line.len() > policy.max_response_bytes {
                return Err(PluginHostError::ResponseTooLarge);
            }
            let response: ProviderPluginResponse = serde_json::from_slice(&response_line)?;
            if response.id != request.id {
                return Err(PluginHostError::ResponseIdMismatch);
            }
            if let Some(error) = response.error {
                return Err(PluginHostError::Plugin {
                    code: error.code,
                    message: error.message,
                });
            }
            Ok(response.result.unwrap_or(Value::Null))
        };

        let result = timeout(policy.timeout, read_response)
            .await
            .map_err(|_| PluginHostError::Timeout)?;
        let _ = child.kill().await;
        result
    }
}

fn command_for_entrypoint(path: &std::path::Path) -> Command {
    #[cfg(windows)]
    {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref()
        {
            Some("cmd" | "bat") => {
                let mut command = Command::new("cmd.exe");
                command.arg("/d").arg("/s").arg("/c").arg(path);
                command
            }
            Some("ps1") => {
                let mut command = Command::new("powershell.exe");
                command
                    .arg("-NoLogo")
                    .arg("-NoProfile")
                    .arg("-NonInteractive")
                    .arg("-ExecutionPolicy")
                    .arg("Bypass")
                    .arg("-File")
                    .arg(path);
                command
            }
            _ => Command::new(path),
        }
    }

    #[cfg(not(windows))]
    {
        Command::new(path)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{collections::BTreeSet, fs, time::Duration};

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::{MANIFEST_FILE_NAME, PluginRegistry};

    #[tokio::test]
    async fn calls_plugin_in_a_separate_process() {
        let root = tempdir().unwrap();
        fs::write(
            root.path().join(MANIFEST_FILE_NAME),
            r#"{
                "schemaVersion":"1.0",
                "id":"dev.example.process-plugin",
                "name":"Process Plugin",
                "version":"1.0.0",
                "entrypoint":"plugin.ps1",
                "capabilities":["chat"],
                "runtimeTypes":["local_runtime"],
                "permissions":[]
            }"#,
        )
        .unwrap();
        fs::write(
            root.path().join("plugin.ps1"),
            r#"$request = ([Console]::In.ReadLine() | ConvertFrom-Json)
$response = @{ id = $request.id; result = @{ isolated = $true } }
[Console]::Out.WriteLine(($response | ConvertTo-Json -Compress))"#,
        )
        .unwrap();

        let plugin = PluginRegistry::load_directory(root.path()).unwrap();
        let result = PluginHost::call(
            &plugin,
            ProviderPluginRequest {
                id: "test-1".into(),
                method: "startSession".into(),
                params: json!({}),
            },
            &PluginHostPolicy {
                approved_permissions: BTreeSet::new(),
                timeout: Duration::from_secs(10),
                max_response_bytes: 1024,
            },
        )
        .await
        .unwrap();

        assert_eq!(result, json!({ "isolated": true }));
    }
}
